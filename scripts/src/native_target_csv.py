"""Thin Python adapter for the canonical Rust target-CSV parser."""

from __future__ import annotations

import csv
import json

from .native_report import produce


def parse_target_csv(csv_text: str) -> dict:
    """Parse target CSV text using the shared report implementation.

    Value errors remain catchable by the path loader so it can emit the
    existing warning. CSV syntax errors preserve the legacy uncaught category.
    """
    response = json.loads(
        produce({"operation": "target_csv_shape", "csv": csv_text})
    )
    if isinstance(response, dict) and isinstance(response.get("target"), dict):
        return response["target"]
    error = response.get("error") if isinstance(response, dict) else None
    if not isinstance(error, dict):
        raise ValueError("target CSV parser returned an invalid response")
    message = error.get("message")
    if not isinstance(message, str):
        raise ValueError("target CSV parser returned an invalid error")
    if error.get("kind") == "csv":
        raise csv.Error(message)
    raise ValueError(message)
