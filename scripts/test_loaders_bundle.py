"""Slim-bundle loader regressions: external curves re-inject transparently."""

import hashlib
import json
import struct
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import scripts.src.loaders as loaders
from scripts.src.loaders import assets_dir_for, load_roomeq_json
from scripts.src.dsp import apply_plugins_to_curve
from scripts.src.payload_binding import ALGORITHM, payload_digest, verify_payload_binding


def _bind_slim(payload):
    body = {k: v for k, v in payload.items() if k != "correction_decisions"}
    payload["correction_decisions"] = {
        "ledger_version": "1.0.0",
        "decisions": [],
        "payload_binding": {
            "algorithm": ALGORITHM,
            "graph_identity": "graph-bundle-1",
            "sha256": payload_digest(body, "graph-bundle-1"),
        },
    }


class BundleLoaderTests(unittest.TestCase):
    def test_room_eq_data_initializes_overlay_flag_and_partial_cleanup_is_safe(self):
        data = loaders.RoomEqData({}, Path("."))
        self.assertFalse(data.deployed_source_curves_created)

        partial = loaders.RoomEqData.__new__(loaders.RoomEqData)
        partial.close()
        partial.__del__()

    def _write_slim(self, root: Path):
        slim = {
            "version": "1",
            "channels": {"L": {"channel": "L", "plugins": []}},
            "deployed_source_curves": {},
        }
        _bind_slim(slim)
        (root / "dsp.json").write_text(json.dumps(slim))
        assets = root / "dsp_files"
        assets.mkdir()
        (assets / "L__initial.csv").write_text("freq,spl,phase\n100,80.0,0.0\n1000,81.0,10.0\n")
        (assets / "L__final.csv").write_text("freq,spl\n100,79.0\n1000,80.0\n")
        (assets / "L__pre_ir.csv").write_text("time_ms,amplitude\n0.0,1.0\n0.1,0.5\n")
        (assets / "deployed__L.csv").write_text("freq,spl\n100,79.5\n1000,80.5\n")
        (assets / "measurements_index.json").write_text(json.dumps({
            "channels": {"L": {
                "initial_curve": "L__initial.csv",
                "final_curve": "L__final.csv",
                "pre_ir": "L__pre_ir.csv",
            }},
            "deployed_source_curves": {"L": "deployed__L.csv"},
            "symmetric_pairs": {"L+R": {"freq": [100.0, 1000.0],
                "sum_spl": [86.0, 87.0], "diff_spl": [None, None],
                "method": "magnitude_sum_log_frequency_interpolation"}},
        }))
        return root / "dsp.json"

    def _write_integrity_manifest(self, json_path: Path):
        assets = assets_dir_for(json_path)
        files = {}
        for member in assets.rglob("*"):
            if not member.is_file():
                continue
            relative = member.relative_to(assets).as_posix()
            if relative in {"artifact_bundle_manifest.json", "manifest.json", "roomeq.log"}:
                continue
            contents = member.read_bytes()
            files[relative] = {
                "size_bytes": len(contents),
                "sha256": hashlib.sha256(contents).hexdigest(),
            }
        (assets / "artifact_bundle_manifest.json").write_text(json.dumps({
            "schema_version": 1,
            "producer": "roomeq-workflow",
            "producer_version": "0.5.33",
            "graph_schema_version": "1",
            "generation": "fixture-generation",
            "graph_sha256": hashlib.sha256(json_path.read_bytes()).hexdigest(),
            "files": files,
        }))

    @staticmethod
    def _float_wav(samples, sample_rate=48000):
        fmt = struct.pack("<HHIIHH", 3, 1, sample_rate, sample_rate * 4, 4, 32)
        pcm = b"".join(struct.pack("<f", sample) for sample in samples)
        chunks = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt
        chunks += b"data" + struct.pack("<I", len(pcm)) + pcm
        return b"RIFF" + struct.pack("<I", len(chunks)) + chunks

    def _write_convolution_graph(self, json_path: Path, reference="resources/test-ir.wav",
                                 inventory=True, resource_exists=True):
        payload = json.loads(json_path.read_text())
        payload["global_plugins"] = [{
            "plugin_type": "convolution",
            "parameters": {"ir_file": reference},
        }]
        content = self._float_wav([0.0, 0.5])
        expected_hash = hashlib.sha256(content).hexdigest()
        if inventory:
            payload["metadata"] = {
                "final_convolution_sha256": {reference: expected_hash},
            }
        _bind_slim(payload)
        json_path.write_text(json.dumps(payload))
        if resource_exists and reference.startswith("resources/"):
            resource = assets_dir_for(json_path) / reference
            resource.parent.mkdir(parents=True, exist_ok=True)
            resource.write_bytes(content)
        return expected_hash

    def test_manifest_verifies_root_and_measurement_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_integrity_manifest(path)
            data = load_roomeq_json(path)
            self.assertEqual(data["channels"]["L"]["initial_curve"]["spl"], [80.0, 81.0])

            (assets_dir_for(path) / "L__initial.csv").write_text("freq,spl\\n100,0\\n")
            with self.assertRaisesRegex(ValueError, "integrity validation"):
                load_roomeq_json(path)

    def test_manifest_detects_native_graph_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_integrity_manifest(path)
            path.write_text(path.read_text().replace('"version": "1"', '"version": "2"'))
            with self.assertRaisesRegex(ValueError, "graph failed artifact bundle integrity"):
                load_roomeq_json(path)

    def test_marked_graph_requires_a_supported_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            payload = json.loads(path.read_text())
            payload["artifact_bundle_schema_version"] = 1
            path.write_text(json.dumps(payload))
            with self.assertRaisesRegex(ValueError, "requires a missing artifact bundle manifest"):
                load_roomeq_json(path)

            payload["artifact_bundle_schema_version"] = 2
            path.write_text(json.dumps(payload))
            with self.assertRaisesRegex(ValueError, "Unsupported artifact bundle schema marker"):
                load_roomeq_json(path)

    def test_manifest_rejects_case_insensitive_duplicate_and_oversized_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_integrity_manifest(path)
            manifest_path = assets_dir_for(path) / "artifact_bundle_manifest.json"
            manifest = json.loads(manifest_path.read_text())
            first_name, first_record = next(iter(manifest["files"].items()))
            manifest["files"][first_name.swapcase()] = first_record
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "case-insensitive duplicate"):
                load_roomeq_json(path)

            manifest_path.write_bytes(b" " * (2 * 1024 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "exceeds its size limit"):
                load_roomeq_json(path)

    def test_mutation_after_manifest_check_is_rejected_during_private_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_integrity_manifest(path)
            original_snapshot = loaders._snapshot_verified_bundle_members

            def mutate_index_then_snapshot(source_assets, verified_members):
                (source_assets / "measurements_index.json").write_text("{}")
                return original_snapshot(source_assets, verified_members)

            with mock.patch.object(
                    loaders, "_snapshot_verified_bundle_members",
                    side_effect=mutate_index_then_snapshot):
                with self.assertRaisesRegex(ValueError, "changed during snapshot"):
                    load_roomeq_json(path)

    def test_manifested_fir_is_parsed_from_a_private_verified_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            payload = json.loads(path.read_text())
            reference = "resources/test-ir.wav"
            wav_bytes = self._float_wav([0.25, -0.5, 0.125])
            payload["channels"]["L"]["drivers"] = [{"plugins": [{
                "plugin_type": "convolution",
                "parameters": {
                    "ir_file": reference,
                    "room_eq_fir_placement": "per_driver",
                },
            }]}]
            payload["metadata"] = {
                "final_convolution_sha256": {reference: hashlib.sha256(wav_bytes).hexdigest()},
            }
            _bind_slim(payload)
            path.write_text(json.dumps(payload))
            source_resource = assets_dir_for(path) / reference
            source_resource.parent.mkdir(parents=True, exist_ok=True)
            source_resource.write_bytes(wav_bytes)
            self._write_integrity_manifest(path)

            original_snapshot = loaders._snapshot_verified_bundle_members

            def mutate_fir_then_snapshot(source_assets, verified_members):
                (source_assets / reference).write_bytes(b"tampered after verification")
                return original_snapshot(source_assets, verified_members)

            with mock.patch.object(
                    loaders, "_snapshot_verified_bundle_members",
                    side_effect=mutate_fir_then_snapshot):
                with self.assertRaisesRegex(ValueError, "changed during snapshot"):
                    load_roomeq_json(path)

            source_resource.write_bytes(wav_bytes)

            data = load_roomeq_json(path)
            snapshot_resource = data.assets_directory / reference
            self.assertNotEqual(data.assets_directory, assets_dir_for(path))
            parameters = data["channels"]["L"]["drivers"][0]["plugins"][0]["parameters"]
            self.assertEqual(parameters._replay_cache["_fir_sample_rate"], 48000)
            self.assertEqual(parameters._replay_cache["_fir_taps"], [0.25, -0.5, 0.125])

            source_resource.write_bytes(b"tampered after load")
            self.assertEqual(snapshot_resource.read_bytes(), wav_bytes)
            replay = apply_plugins_to_curve(
                {"freq": [12_000.0], "spl": [80.0], "phase": [0.0]},
                data["channels"]["L"]["drivers"][0]["plugins"],
            )
            self.assertAlmostEqual(replay["spl"][0], 74.24268947394387, delta=1e-10)
            self.assertAlmostEqual(replay["phase"][0], 75.96375653207353, delta=1e-10)
            snapshot_directory = data.assets_directory
            data.close()
            self.assertFalse(snapshot_directory.exists())
            self.assertTrue(source_resource.exists())

    def test_manifest_binds_every_convolution_resource(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            digest = self._write_convolution_graph(path)
            self._write_integrity_manifest(path)

            data = load_roomeq_json(path)
            self.assertEqual(
                data["metadata"]["final_convolution_sha256"]["resources/test-ir.wav"], digest)

            resource = assets_dir_for(path) / "resources/test-ir.wav"
            resource.write_bytes(b"tampered FIR bytes")
            with self.assertRaisesRegex(ValueError, "member failed integrity validation"):
                load_roomeq_json(path)

    def test_shared_driver_and_global_firs_replay_frozen_manifested_taps(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_convolution_graph(path)
            payload = json.loads(path.read_text())
            plugin = payload["global_plugins"][0]
            channel = payload["channels"]["L"]
            channel["plugins"] = [plugin]
            channel["drivers"] = [{"plugins": [plugin]}]
            _bind_slim(payload)
            path.write_text(json.dumps(payload))
            self._write_integrity_manifest(path)
            with mock.patch.object(loaders, "read_fir_wav", wraps=loaders.read_fir_wav) as read:
                data = load_roomeq_json(path)
            read.assert_called_once()
            resource = assets_dir_for(path) / "resources/test-ir.wav"
            resource.write_bytes(b"changed after loading")
            self.assertTrue(verify_payload_binding(data)[0])
            chains = [data["global_plugins"], data["channels"]["L"]["plugins"],
                      data["channels"]["L"]["drivers"][0]["plugins"]]
            for chain in chains:
                with self.subTest(chain=chain):
                    self.assertNotIn("_fir_taps", chain[0]["parameters"])
                    corrected = apply_plugins_to_curve(
                        {"freq": [12000.0], "spl": [80.0], "phase": [0.0]}, chain)
                    self.assertAlmostEqual(corrected["spl"][0], 73.9794000867, places=8)
                    self.assertAlmostEqual(corrected["phase"][0], -90.0)
            data.close()

    def test_manifested_convolution_requires_exact_graph_hash_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_convolution_graph(path, inventory=False)
            self._write_integrity_manifest(path)
            with self.assertRaisesRegex(ValueError, "inventory does not exactly match"):
                load_roomeq_json(path)

        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_convolution_graph(
                path, reference="resources/missing.wav", resource_exists=False)
            self._write_integrity_manifest(path)
            with self.assertRaisesRegex(ValueError, "not a bundled file"):
                load_roomeq_json(path)

    def test_manifested_convolution_digest_must_match_bundle_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_convolution_graph(path)
            payload = json.loads(path.read_text())
            payload["metadata"]["final_convolution_sha256"]["resources/test-ir.wav"] = "0" * 64
            _bind_slim(payload)
            path.write_text(json.dumps(payload))
            self._write_integrity_manifest(path)

            with self.assertRaisesRegex(ValueError, "does not match its SHA-256 binding"):
                load_roomeq_json(path)

    def test_manifested_convolution_rejects_parent_traversal(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_convolution_graph(path, reference="../outside.wav", resource_exists=False)
            self._write_integrity_manifest(path)

            with self.assertRaisesRegex(ValueError, "Invalid bundle member path"):
                load_roomeq_json(path)

    def test_legacy_graph_may_keep_external_convolution_reference(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            self._write_convolution_graph(path, reference="/outside/legacy-fir.wav")
            self.assertEqual(load_roomeq_json(path)["global_plugins"][0]["parameters"]["ir_file"],
                             "/outside/legacy-fir.wav")

    def test_pending_transaction_refuses_partial_python_load(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            journal = path.with_name(path.name + ".autoeq-transaction.json")
            journal.write_text("{}")
            with self.assertRaisesRegex(RuntimeError, "publication is incomplete"):
                load_roomeq_json(path)

    def test_pending_external_export_transaction_refuses_partial_python_load(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            journal = path.with_name(f".{path.name}.external-export-journal.json")
            journal.write_text("{}")
            with self.assertRaisesRegex(RuntimeError, "native/export publication is incomplete"):
                load_roomeq_json(path)

    def test_external_curves_reinject_with_phase_and_ir(self):
        with tempfile.TemporaryDirectory() as directory:
            data = load_roomeq_json(self._write_slim(Path(directory)))
            self.assertEqual(data["channels"]["L"]["initial_curve"]["spl"], [80.0, 81.0])
            self.assertEqual(data["channels"]["L"]["initial_curve"]["phase"], [0.0, 10.0])
            self.assertEqual(data["channels"]["L"]["final_curve"]["spl"], [79.0, 80.0])
            self.assertNotIn("phase", data["channels"]["L"]["final_curve"])
            self.assertEqual(data["channels"]["L"]["pre_ir"]["amplitude"], [1.0, 0.5])
            self.assertEqual(data["deployed_source_curves"]["L"]["spl"], [79.5, 80.5])
            self.assertEqual(data.symmetric_pairs['L+R']['sum_spl'], [86.0, 87.0])
            self.assertNotIn('symmetric_pairs', data)

    def test_ir64_bundle_loads_full_rate_bits_instead_of_display_csv(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            assets = assets_dir_for(path)
            samples = [(-0.1234567890123 + index / 48.0, -1.234567890123456e-120)
                       for index in range(10_000)]
            samples[-1] = (samples[-1][0], -0.0)
            raw = b"AEQIR64\0" + struct.pack("<Q", len(samples))
            raw += b"".join(struct.pack("<dd", *sample) for sample in samples)
            (assets / "L__pre_ir.ir64").write_bytes(raw)
            (assets / "L__pre_ir_display.csv").write_text("time_ms,amplitude\n0,99\n")
            index_path = assets / "measurements_index.json"
            index = json.loads(index_path.read_text())
            index["channels"]["L"]["pre_ir"] = "L__pre_ir.ir64"
            index["channels"]["L"]["pre_ir_display"] = "L__pre_ir_display.csv"
            index_path.write_text(json.dumps(index))
            self._write_integrity_manifest(path)
            data = load_roomeq_json(path)
            ir = data["channels"]["L"]["pre_ir"]
            self.assertEqual(len(ir["time_ms"]), len(samples))
            decoded = b"".join(struct.pack("<dd", time, value)
                               for time, value in zip(ir["time_ms"], ir["amplitude"]))
            self.assertEqual(decoded, raw[16:])

    def test_ir64_reader_refuses_bad_counts_and_nonfinite_samples(self):
        malformed = [b"AEQIR64\0", b"AEQIR64\0" + struct.pack("<Q", 2**64 - 1),
                     b"AEQIR64\0" + struct.pack("<Qd", 1, 0.0),
                     b"AEQIR64\0" + struct.pack("<Qdd", 1, 0.0, float("nan"))]
        for raw in malformed:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                loaders.read_ir_csv(raw)

    def test_curve_metadata_sidecars_preserve_all_native_curve_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._write_slim(Path(directory))
            assets = assets_dir_for(path)
            curve = {
                "freq": [123.45678901234567, 2345.678901234567],
                "spl": [80.12345678901235, 81.23456789012345],
                "phase": [12.3456789012345, -23.456789012345],
                "norm_range": [100.1234567890123, 19000.98765432109],
                "noise_floor_db": [20.12345678901235, 21.23456789012345],
                "coherence": [0.9123456789012345, 0.9876543210987654],
            }
            (assets / "L__initial.csv.json").write_text(json.dumps(curve))
            (assets / "deployed__L.csv.json").write_text(json.dumps(curve))
            index_path = assets / "measurements_index.json"
            index = json.loads(index_path.read_text())
            index["channels"]["L"]["initial_curve_metadata"] = "L__initial.csv.json"
            index["deployed_source_curve_metadata"] = {"L": "deployed__L.csv.json"}
            index_path.write_text(json.dumps(index))
            self._write_integrity_manifest(path)

            data = load_roomeq_json(path)
            self.assertEqual(data["channels"]["L"]["initial_curve"], curve)
            self.assertEqual(data["deployed_source_curves"]["L"], curve)

    def test_manifested_curve_metadata_rejects_malformed_shapes_and_values(self):
        valid = {
            "freq": [123.45678901234567, 2345.678901234567],
            "spl": [80.0, 81.0],
            "phase": [0.0, 1.0],
            "norm_range": [100.0, 20000.0],
            "noise_floor_db": [20.0, 21.0],
            "coherence": [0.9, 0.95],
        }
        invalid_curves = []
        bad_length = dict(valid, noise_floor_db=[20.0])
        invalid_curves.append((bad_length, "noise_floor_db length"))
        bad_values = dict(valid, spl=[80.0, float("nan")])
        invalid_curves.append((bad_values, "finite numbers"))
        bad_coherence = dict(valid, coherence=[0.9, 1.2])
        invalid_curves.append((bad_coherence, "coherence values"))
        bad_range = dict(valid, norm_range=[20000.0, 100.0])
        invalid_curves.append((bad_range, "norm_range must be positive and ordered"))

        for curve, error in invalid_curves:
            with self.subTest(error=error), tempfile.TemporaryDirectory() as directory:
                path = self._write_slim(Path(directory))
                assets = assets_dir_for(path)
                metadata = "L__initial.csv.json"
                (assets / metadata).write_text(json.dumps(curve))
                index_path = assets / "measurements_index.json"
                index = json.loads(index_path.read_text())
                index["channels"]["L"]["initial_curve_metadata"] = metadata
                index_path.write_text(json.dumps(index))
                self._write_integrity_manifest(path)

                with self.assertRaisesRegex(ValueError, error):
                    load_roomeq_json(path)

    def test_slim_binding_verifies_through_the_overlay(self):
        with tempfile.TemporaryDirectory() as directory:
            data = load_roomeq_json(self._write_slim(Path(directory)))
            valid, reason, _ = verify_payload_binding(data)
            self.assertTrue(valid, reason)

    def test_absent_deployed_key_is_left_untouched_without_index(self):
        # Rust omits `deployed_source_curves` when empty; the loader must
        # not invent the key or verification hashes different bytes.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            slim = {
                "version": "1",
                "channels": {"L": {"channel": "L", "plugins": []}},
            }
            _bind_slim(slim)
            path = root / "nodeployed.json"
            path.write_text(json.dumps(slim))
            data = load_roomeq_json(path)
            self.assertNotIn("deployed_source_curves", data)
            valid, reason, _ = verify_payload_binding(data)
            self.assertTrue(valid, reason)

    def test_absent_deployed_key_verifies_through_full_extraction(self):
        # Fully extracted slim output: the key was emptied out of the saved
        # bytes, so stripping the overlay must drop it again for hashing.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            slim = {
                "version": "1",
                "channels": {"L": {"channel": "L", "plugins": []}},
            }
            _bind_slim(slim)
            (root / "dsp.json").write_text(json.dumps(slim))
            assets = root / "dsp_files"
            assets.mkdir()
            (assets / "deployed__L.csv").write_text("freq,spl\n100,79.5\n1000,80.5\n")
            (assets / "measurements_index.json").write_text(json.dumps({
                "channels": {},
                "deployed_source_curves": {"L": "deployed__L.csv"},
            }))
            data = load_roomeq_json(root / "dsp.json")
            self.assertEqual(data["deployed_source_curves"]["L"]["spl"], [79.5, 80.5])
            valid, reason, _ = verify_payload_binding(data)
            self.assertTrue(valid, reason)

    def test_legacy_embedded_output_loads_without_overlay(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "legacy.json"
            path.write_text(json.dumps({
                "channels": {"L": {
                    "channel": "L",
                    "plugins": [],
                    "initial_curve": {"freq": [100.0], "spl": [80.0]},
                }},
            }))
            data = load_roomeq_json(path)
            self.assertEqual(
                data["channels"]["L"]["initial_curve"], {"freq": [100.0], "spl": [80.0]})
            self.assertEqual(data.measurement_index, {})


if __name__ == "__main__":
    unittest.main()
