"""Adapters for canonical saved-evidence report sections."""

import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


def evidence_section(builder, **arguments):
    """Render saved evidence with the shared native/WASM report producer."""
    request = {"operation": "evidence_section", "builder": builder}
    request.update(_json_safe(arguments))
    return json.loads(produce(request))
