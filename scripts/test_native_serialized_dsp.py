#!/usr/bin/env python3

"""Parity tests for canonical serialized EQ response evaluation."""

import json
import math
from pathlib import Path
import unittest

from scripts.src.dsp import serialized_filter_response


FIXTURE = Path(__file__).parent / "tests/fixtures/serialized_dsp_filter_response_goldens.json"


class NativeSerializedDspTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_matches_pre_port_python_and_engine_goldens(self):
        for case in self.fixture["cases"]:
            if case.get("unavailable"):
                with self.subTest(case=case["name"]):
                    with self.assertRaises(ValueError):
                        serialized_filter_response(case["filter"], case["frequencies"], case["sample_rate"])
                continue

            actual = serialized_filter_response(case["filter"], case["frequencies"], case["sample_rate"])
            self.assertEqual(len(actual), len(case["frequencies"]))
            for index, response in enumerate(actual):
                with self.subTest(case=case["name"], index=index):
                    for oracle in (case["python"], case["engine"]):
                        self.assertAlmostEqual(response.real, oracle[index][0], delta=1e-12)
                        self.assertAlmostEqual(response.imag, oracle[index][1], delta=1e-12)

    def test_legacy_filter_aliases_and_defaults_are_normalized_before_rust(self):
        canonical = serialized_filter_response(
            {"filter_type": "peak", "freq": 900.0, "q": 1.2, "db_gain": -3.0},
            [0.0, 900.0, 8000.0],
            48_000.0,
        )
        aliases = serialized_filter_response(
            {"filter_type": "Peaking", "frequency": 900.0, "q": 1.2, "gain": -3.0},
            [0.0, 900.0, 8000.0],
            48_000.0,
        )
        self.assertEqual(aliases, canonical)

        defaulted = serialized_filter_response({}, [0.0, 1000.0, 8000.0], 48_000.0)
        explicit = serialized_filter_response(
            {"filter_type": "peak", "freq": 1000.0, "q": 1.0, "db_gain": 0.0},
            [0.0, 1000.0, 8000.0],
            48_000.0,
        )
        self.assertEqual(defaulted, explicit)

    def test_invalid_grid_and_non_finite_values_remain_unavailable(self):
        filter_value = {"filter_type": "peak", "freq": 1000.0, "q": 1.0, "db_gain": 0.0}
        for frequencies, rate in (([math.nan], 48_000.0), ([-1.0], 48_000.0), ([24_001.0], 48_000.0), ([100.0], math.inf)):
            with self.subTest(frequencies=frequencies, rate=rate):
                with self.assertRaises(ValueError):
                    serialized_filter_response(filter_value, frequencies, rate)

    def test_invalid_explicit_topology_and_parameters_do_not_default(self):
        for filt in (
            {"topology": "unknown"},
            {"filter_type": "peak", "freq": 1000.0, "q": None, "db_gain": 0.0},
            {"topology": "warped_biquad", "filter_type": "peak", "freq": 1000.0, "q": 1.0, "db_gain": 0.0, "lambda": 1.0},
        ):
            with self.subTest(filter=filt):
                with self.assertRaises((TypeError, ValueError)):
                    serialized_filter_response(filt, [500.0], 48_000.0)


if __name__ == "__main__":
    unittest.main()
