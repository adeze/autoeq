"""Tests for the native impulse-response group-delay adapter."""

import json
import math
import unittest

from scripts.src import native_ir_group_delay
from scripts.src.native_report import using_producer


class NativeIrGroupDelayTests(unittest.TestCase):
    def test_dispatches_waveform_settings_and_returns_tuple(self):
        requests = []

        def producer(request):
            requests.append(request)
            return json.dumps({"frequency_hz": [100.0, 200.0], "group_delay_ms": [1.5, 1.25]})

        ir = {
            "time_ms": [0.0, 0.1, 0.2, 0.3],
            "amplitude": [0.0, math.nan, 1.0, 0.0],
        }
        with using_producer(producer):
            result = native_ir_group_delay.compute_group_delay_from_ir(
                ir, min_freq=30.0, max_freq=9_000.0, point_count=64.8
            )

        self.assertEqual(result, ([100.0, 200.0], [1.5, 1.25]))
        self.assertEqual(requests, [{
            "operation": "ir_group_delay",
            "time_ms": [0.0, 0.1, 0.2, 0.3],
            "amplitude": [0.0, None, 1.0, 0.0],
            "min_freq": 30.0,
            "max_freq": 9_000.0,
            "point_count": 64,
        }])

    def test_absent_impulse_response_returns_empty_tuple_without_producer(self):
        def fail_if_called(_request):
            self.fail("absent IR should not start the native report producer")

        with using_producer(fail_if_called):
            self.assertEqual(native_ir_group_delay.compute_group_delay_from_ir(None), ([], []))


if __name__ == "__main__":
    unittest.main()
