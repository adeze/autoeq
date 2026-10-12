"""Thin adapters for canonical serialized report data extraction."""

import json
from .native_report import produce
from .native_acoustic_t60 import _json_safe


def helper(name, **arguments):
    return json.loads(produce(_json_safe({"operation": "report_data", "helper": name, **arguments})))
