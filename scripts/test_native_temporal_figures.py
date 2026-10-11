"""Contract checks for the native temporal-figure request adapters."""

import json
import unittest

from scripts.src.native_report import using_producer
from scripts.src.native_temporal_figures import (
    create_early_late_figure,
    create_ir_figure,
    create_symmetric_pair_figure,
    create_t60_octaves_figure,
    create_tof_figure,
    finite_or_gap,
    to_db,
)


class NativeTemporalFigureAdapterTests(unittest.TestCase):
    def test_figure_functions_forward_named_arguments_to_canonical_builder(self):
        requests = []

        def producer(request):
            requests.append(request)
            return json.dumps({"kind": "figure", "tab": request.get("tab")})

        with using_producer(producer):
            self.assertEqual(
                create_ir_figure("L", {"time_ms": [0], "amplitude": [1]}, None, 75, "ir"),
                {"kind": "figure", "tab": "ir"},
            )
            create_tof_figure([], after=True, tab="tof")
            create_early_late_figure("L", {"method": "incoherent_band_energy"})
            create_t60_octaves_figure("Room", [], window=[0.2, 0.4])
            create_symmetric_pair_figure("Pair", [100], [70], [68], "pair")
            to_db([1.0, 0.0])
            finite_or_gap([1.0, None])

        self.assertEqual(
            requests,
            [
                {
                    "operation": "temporal_figure",
                    "builder": "create_ir_figure",
                    "channel_name": "L",
                    "pre_ir": {"time_ms": [0], "amplitude": [1]},
                    "post_ir": None,
                    "display_ms": 75,
                    "tab": "ir",
                },
                {
                    "operation": "temporal_figure",
                    "builder": "create_tof_figure",
                    "tof_rows": [],
                    "after": True,
                    "tab": "tof",
                },
                {
                    "operation": "temporal_figure",
                    "builder": "create_early_late_figure",
                    "label": "L",
                    "report": {"method": "incoherent_band_energy"},
                    "tab": None,
                },
                {
                    "operation": "temporal_figure",
                    "builder": "create_t60_octaves_figure",
                    "label": "Room",
                    "rows": [],
                    "tab": None,
                    "window": [0.2, 0.4],
                    "itu_reference": None,
                },
                {
                    "operation": "temporal_figure",
                    "builder": "create_symmetric_pair_figure",
                    "label": "Pair",
                    "freq": [100],
                    "abs_spl": [70],
                    "complex_spl": [68],
                    "tab": "pair",
                },
                {"operation": "temporal_figure", "builder": "to_db", "amplitude": [1.0, 0.0]},
                {"operation": "temporal_figure", "builder": "finite_or_gap", "values": [1.0, None]},
            ],
        )

    def test_unavailable_figure_and_gap_inputs_are_not_hydrated_in_python(self):
        requests = []

        def producer(request):
            requests.append(request)
            return "null"

        with using_producer(producer):
            result = create_ir_figure("L", None, None)
            create_tof_figure([{"name": "L", "before_ms": float("nan")}])

        self.assertIsNone(result)
        self.assertIsNone(requests[0]["pre_ir"])
        self.assertIsNone(requests[1]["tof_rows"][0]["before_ms"])


if __name__ == "__main__":
    unittest.main()
