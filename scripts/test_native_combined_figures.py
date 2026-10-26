"""Parity tests for the canonical combined overview figure payload."""

import json
import math
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts.src import native_combined_figures
from scripts.src.figures import create_combined_figure


ROOT = Path(__file__).resolve().parent
GOLDENS = ROOT / "tests" / "fixtures" / "combined_figures_python_goldens.json"


def _curve(base):
    return {
        "freq": [30.0, 80.0, 250.0, 1000.0],
        "spl": [base, base + 2, base - 1, base + 1],
    }


def _fixture_data():
    return {
        "sample_rate": 48000,
        "channels": {
            "R": {
                "initial_curve": _curve(70),
                "final_curve": _curve(72),
                "eq_response": {
                    "freq": [30.0, 100.0, 1000.0],
                    "spl": [1.0, -2.0, 0.5],
                },
            },
            "LFE": {
                "initial_curve": _curve(61),
                "final_curve": _curve(60),
                "eq_response": {
                    "freq": [30.0, 100.0, 1000.0],
                    "spl": [-1.0, -4.0, 0.0],
                },
            },
            "L": {
                "initial_curve": _curve(69),
                "final_curve": _curve(71),
                "eq_response": {
                    "freq": [30.0, 100.0, 1000.0],
                    "spl": [0.0, -1.0, 0.25],
                },
            },
        },
        "metadata": {
            "bass_management": {
                "physical_sub_output": "LFE",
                "routing_graph": {"routes": []},
            }
        },
    }


class NativeCombinedFigureTests(unittest.TestCase):
    def test_native_sections_match_pre_port_python_goldens(self):
        expected = json.loads(GOLDENS.read_text(encoding="utf-8"))
        self.assertEqual(create_combined_figure(_fixture_data()), expected["basic"])
        self.assertEqual(create_combined_figure({"channels": {}}), expected["empty"])

    def test_adapter_uses_combined_operation_and_returns_json_sections(self):
        requests = []

        def produce(request):
            requests.append(request)
            return '[{"kind":"figure"}]'

        prepared = {"has_channels": True, "before_series": []}
        with patch.object(native_combined_figures, "produce", side_effect=produce):
            result = native_combined_figures.combined_figures(prepared, "Summary")

        self.assertEqual(result, [{"kind": "figure"}])
        self.assertEqual(requests[0]["operation"], "combined_figures")
        self.assertEqual(requests[0]["prepared"], prepared)
        self.assertEqual(requests[0]["tab"], "Summary")

    def test_adapter_normalizes_nonfinite_prepared_bins(self):
        seen = []

        def produce(request):
            seen.append(request)
            return "[]"

        with patch.object(native_combined_figures, "produce", side_effect=produce):
            native_combined_figures.combined_figures(
                {"has_channels": True, "before_series": [{"x": [math.inf], "y": [math.nan]}]}
            )

        self.assertIsNone(seen[0]["prepared"]["before_series"][0]["x"][0])
        self.assertIsNone(seen[0]["prepared"]["before_series"][0]["y"][0])
        json.dumps(seen, allow_nan=False)


if __name__ == "__main__":
    unittest.main()
