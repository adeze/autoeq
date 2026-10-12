"""Tests for scoped report-producer transport without a subprocess."""

import json
import math
import os
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from scripts.src import native_report


class ReportTransportTests(unittest.TestCase):
    def test_callback_receives_strict_json_request_and_returns_native_stdout(self):
        callback = Mock(return_value='{"ok":true}\n')
        request = {"label": "room Ω", "data": {"measurement": [1.0, None]}}

        with patch.object(native_report.subprocess, "run") as run:
            with native_report.using_producer(callback):
                result = native_report.produce(request)
            run.assert_not_called()

        self.assertEqual(result, '{"ok":true}\n')
        callback.assert_called_once_with(request)

    def test_nested_overrides_restore_the_outer_callback(self):
        calls = []

        def outer(request):
            calls.append(("outer", request))
            return "outer-result"

        def inner(request):
            calls.append(("inner", request))
            return "inner-result"

        with native_report.using_producer(outer):
            self.assertEqual(native_report.produce({"n": 1}), "outer-result")
            with native_report.using_producer(inner):
                self.assertEqual(native_report.produce({"n": 2}), "inner-result")
            self.assertEqual(native_report.produce({"n": 3}), "outer-result")

        self.assertEqual([name for name, _ in calls], ["outer", "inner", "outer"])

    def test_override_resets_after_context_and_callback_exceptions(self):
        marker = RuntimeError("callback failed")
        callback = Mock(side_effect=marker)
        with self.assertRaisesRegex(RuntimeError, "callback failed"):
            with native_report.using_producer(callback):
                with self.assertRaisesRegex(RuntimeError, "callback failed"):
                    native_report.produce({"request": 1})
                raise marker

        run_result = SimpleNamespace(returncode=0, stdout="fallback", stderr="")
        with patch.dict(os.environ, {"AUTOEQ_REPORT_PRODUCER": "/test/report-producer"}), \
             patch.object(native_report.subprocess, "run", return_value=run_result) as run:
            self.assertEqual(native_report.produce({"request": 2}), "fallback")
        self.assertEqual(run.call_args.kwargs["input"], '{"request": 2}')

    def test_invalid_json_never_reaches_callback(self):
        callback = Mock(return_value="ignored")
        with native_report.using_producer(callback):
            with self.assertRaises(ValueError):
                native_report.produce({"measurement": math.nan})
            with self.assertRaises(TypeError):
                native_report.produce({"measurement": object()})
        callback.assert_not_called()

    def test_callback_errors_and_invalid_return_types_propagate(self):
        marker = LookupError("host producer failed")
        with native_report.using_producer(Mock(side_effect=marker)):
            with self.assertRaises(LookupError) as raised:
                native_report.produce({"request": True})
        self.assertIs(raised.exception, marker)

        with native_report.using_producer(lambda _request: None):
            with self.assertRaisesRegex(TypeError, "must return a string"):
                native_report.produce({"request": True})


if __name__ == "__main__":
    unittest.main()
