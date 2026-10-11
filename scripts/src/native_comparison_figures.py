"""Batched adapter for canonical comparison figure-section assembly."""

from __future__ import annotations

from contextvars import ContextVar
from dataclasses import dataclass
import json

from .native_acoustic_t60 import _json_safe
from .native_report import produce


_pending_specs: ContextVar[list[dict] | None] = ContextVar(
    "autoeq_comparison_figure_specs", default=None
)


@dataclass(frozen=True)
class _PendingFigure:
    index: int


def comparison_figure_section(spec):
    """Queue a prepared spec in a batch or render one standalone figure."""
    batch = _pending_specs.get()
    if batch is not None:
        index = len(batch)
        batch.append(spec)
        return _PendingFigure(index)
    return comparison_figure_sections([spec])[0]


def comparison_figure_sections(specs):
    """Render prepared comparison figure specs with one producer call."""
    return json.loads(produce({
        "operation": "comparison_figure_sections",
        "prepared": _json_safe({"figures": specs}),
    }))


class ComparisonFigureBatch:
    """Collect multiple comparison figures for one producer round trip."""

    def __init__(self):
        self.specs = []
        self._token = None
        self._sections = None

    def __enter__(self):
        self._token = _pending_specs.set(self.specs)
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        _pending_specs.reset(self._token)

    def resolve(self, value):
        if self._sections is None:
            self._sections = comparison_figure_sections(self.specs) if self.specs else []
        if isinstance(value, _PendingFigure):
            return self._sections[value.index]
        if isinstance(value, list):
            return [self.resolve(item) for item in value]
        return value
