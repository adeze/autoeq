"""Contract tests for the host-backed report producer transport."""

from __future__ import annotations

import json
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from scripts.src.host_report_transport import file_producer


class HostReportTransportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def respond(self, producer, request, result="{\"value\":1}", response_id=None):
        request_path = self.directory / "request.json"
        response_path = self.directory / "response.json"
        returned = {}

        def invoke():
            try:
                returned["output"] = producer(request)
            except Exception as error:  # test thread reports back through this map
                returned["error"] = error

        thread = threading.Thread(target=invoke, daemon=True)
        thread.start()
        envelope = self.wait_for_request(request_path)
        self.assertEqual(envelope["request"], request)
        request_path.unlink(missing_ok=True)
        self.write_response(response_path, {
            "id": envelope["id"] if response_id is None else response_id,
            "ok": True,
            "output": result,
        })
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        if "error" in returned:
            raise returned["error"]
        return envelope["id"], returned["output"]

    @staticmethod
    def wait_for_request(request_path):
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline:
            if request_path.exists():
                return json.loads(request_path.read_text(encoding="utf-8"))
            time.sleep(0.002)
        raise AssertionError("producer did not publish a request")

    @staticmethod
    def write_response(response_path, response):
        temporary = response_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(response), encoding="utf-8")
        temporary.replace(response_path)

    def test_atomic_envelope_echoes_monotonic_ids_and_returns_host_output(self):
        producer = file_producer(self.directory)
        first_id, output = self.respond(producer, {"operation": "summary", "label": "Ω"})
        self.assertEqual(first_id, 1)
        self.assertEqual(output, '{"value":1}')
        second_id, _ = self.respond(producer, {"operation": "summary"})
        self.assertEqual(second_id, 2)
        self.assertFalse((self.directory / "response.json").exists())
        self.assertFalse(list(self.directory.glob("request-*.tmp")))

    def test_response_id_must_match_outstanding_request(self):
        producer = file_producer(self.directory)
        with self.assertRaisesRegex(RuntimeError, "ID mismatch"):
            self.respond(producer, {"operation": "summary"}, response_id=99)

    def test_host_error_is_propagated_with_actionable_context(self):
        request_path = self.directory / "request.json"
        response_path = self.directory / "response.json"
        result = {}

        def invoke():
            try:
                file_producer(self.directory, timeout_s=1)({"operation": "bad"})
            except Exception as error:
                result["error"] = error

        thread = threading.Thread(target=invoke, daemon=True)
        thread.start()
        envelope = self.wait_for_request(request_path)
        request_path.unlink(missing_ok=True)
        self.write_response(response_path, {
            "id": envelope["id"], "ok": False, "error": "unknown operation"
        })
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertRegex(str(result["error"]), r"request 1 failed: unknown operation")

    def test_timeout_cleans_unhandled_request(self):
        producer = file_producer(self.directory, timeout_s=0.01)
        with self.assertRaisesRegex(TimeoutError, "request 1"):
            producer({"operation": "summary"})
        self.assertFalse((self.directory / "request.json").exists())

    def test_request_and_response_limits_can_be_checked_without_large_allocations(self):
        with patch("scripts.src.host_report_transport.MAX_MESSAGE_BYTES", 32):
            producer = file_producer(self.directory)
            with self.assertRaisesRegex(ValueError, "request.*limit is 32 bytes"):
                producer({"large": "x" * 100})
            response_producer = file_producer(self.directory)
            result = {}

            def invoke():
                try:
                    response_producer({})
                except Exception as error:
                    result["error"] = error

            thread = threading.Thread(target=invoke, daemon=True)
            thread.start()
            request_path = self.directory / "request.json"
            envelope = self.wait_for_request(request_path)
            request_path.unlink(missing_ok=True)
            self.write_response(self.directory / "response.json", {
                "id": envelope["id"], "ok": True, "output": "x" * 100
            })
            thread.join(timeout=1)
            self.assertFalse(thread.is_alive())
            self.assertRegex(str(result["error"]), "response.*limit is 32 bytes")

    def test_strict_json_rejects_nonfinite_request_numbers(self):
        producer = file_producer(self.directory)
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "strict JSON"):
                producer({"measurement": value})
        self.assertFalse((self.directory / "request.json").exists())

    def test_response_json_rejects_nonfinite_constants(self):
        from scripts.src import host_report_transport

        with self.assertRaisesRegex(RuntimeError, "non-finite JSON number NaN"):
            host_report_transport._decode_response(
                b'{"id":1,"ok":true,"output":"x","extra":NaN}', 1
            )

    def test_single_callback_serializes_concurrent_calls(self):
        producer = file_producer(self.directory)
        outputs = []
        errors = []

        def invoke(index):
            try:
                outputs.append(producer({"index": index}))
            except Exception as error:
                errors.append(error)

        threads = [threading.Thread(target=invoke, args=(index,), daemon=True) for index in range(2)]
        for thread in threads:
            thread.start()
        request_path = self.directory / "request.json"
        response_path = self.directory / "response.json"
        observed = []
        for _ in range(2):
            envelope = self.wait_for_request(request_path)
            observed.append(envelope["id"])
            request_path.unlink(missing_ok=True)
            self.write_response(response_path, {"id": envelope["id"], "ok": True, "output": "ok"})
            deadline = time.monotonic() + 1
            while response_path.exists() and time.monotonic() < deadline:
                time.sleep(0.002)
        for thread in threads:
            thread.join(timeout=1)
            self.assertFalse(thread.is_alive())
        self.assertEqual(observed, [1, 2])
        self.assertEqual(len(outputs), 2)
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
