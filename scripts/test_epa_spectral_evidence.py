"""Frozen target-error EPA receipts remain independent of transfer descriptors."""
import copy
import json
import unittest
from epa_evidence import inspect_epa_spectral_flatness


class EpaSpectralEvidenceTests(unittest.TestCase):
    def fixture(self):
        basis = {"frequency_grid_sha256": "a" * 64, "baseline_samples_sha256": "b" * 64,
                 "target_samples_sha256": "c" * 64, "sample_count": 128,
                 "evaluated_band_hz": [20.0, 20000.0], "normalized_to_pre": True,
                 "level_offset_db": 80.0, "post_reference_gain_db": 0.0,
                 "post_covers_baseline_grid": True}
        detail = dict(basis, method="configured_frozen_epa_spectral_flatness_v1",
                      scope="target_spectral_error_not_measured_listener_preference",
                      comparison="at_most", unit="dB", pre_loss_db=4.0, post_loss_db=3.5,
                      signed_violation_margin=-0.5-1e-9, erb_weight=1.0, band_weight=0.0,
                      band_weights={"bass_weight": 2.0, "mid_weight": 1.0, "treble_weight": 0.8})
        check = dict(id="epa_spectral_flatness", kind="quality", passed=True,
                     observed=-0.5, limit=1e-9, diagnostic=json.dumps(detail))
        residual = dict(seat_index=0, score=basis, epa_spectral_flatness=check)
        metadata = {"stage_outcomes": [{"stage": "delivered_epa_non_regression", "advisories": [
            "criterion=configured_frozen_epa_spectral_flatness_v1"]}], "correction_acceptance": {
            "outcome": "accepted", "acoustic_quality": {"training": {"configured_baseline": {
                "residuals": [residual]}}}}}
        return metadata, residual, detail

    def test_checks_arithmetic_basis_and_missing_accepted_evidence(self):
        metadata, residual, detail = self.fixture()
        self.assertTrue(inspect_epa_spectral_flatness(metadata)["rows"][0]["assessment"]["passed"])
        for key, value in [("post_loss_db", 9.0), ("frequency_grid_sha256", "d" * 64),
                           ("level_offset_db", 82.0), ("erb_weight", -1.0)]:
            with self.subTest(key=key):
                changed = copy.deepcopy(detail)
                changed[key] = value
                residual["epa_spectral_flatness"]["diagnostic"] = json.dumps(changed)
                with self.assertRaises(ValueError):
                    inspect_epa_spectral_flatness(metadata)
        residual["epa_spectral_flatness"]["diagnostic"] = json.dumps(detail)
        residual["epa_spectral_flatness"] = None
        with self.assertRaisesRegex(ValueError, "complete non-regressing"):
            inspect_epa_spectral_flatness(metadata)
        metadata["correction_acceptance"]["outcome"] = "unchanged"
        self.assertIsNone(inspect_epa_spectral_flatness(metadata)["rows"][0]["assessment"])
        self.assertIsNone(inspect_epa_spectral_flatness({}))

    def test_failed_regression_is_reported_and_cannot_be_claimed_accepted(self):
        metadata, residual, detail = self.fixture()
        detail.update(post_loss_db=4.1, signed_violation_margin=0.1-1e-9)
        residual["epa_spectral_flatness"].update(passed=False, observed=0.1, diagnostic=json.dumps(detail))
        metadata["correction_acceptance"]["outcome"] = "safe_fallback"
        self.assertFalse(inspect_epa_spectral_flatness(metadata)["rows"][0]["assessment"]["passed"])
        metadata["correction_acceptance"]["outcome"] = "accepted"
        with self.assertRaisesRegex(ValueError, "complete non-regressing"):
            inspect_epa_spectral_flatness(metadata)
