"""Thin JSON bridge to the canonical native/WASM report producer."""

import json
import os
from pathlib import Path
import shutil
import subprocess
from contextlib import contextmanager
from contextvars import ContextVar
from typing import Callable, Iterator, Optional


_producer_override: ContextVar[Optional[Callable[[object], str]]] = ContextVar(
    "autoeq_native_report_producer", default=None
)


@contextmanager
def using_producer(callback: Callable[[object], str]) -> Iterator[None]:
    """Temporarily route report requests through a context-local producer.

    The override is scoped to the current context and restored even if the
    callback or the context body raises. Outside this scope, ``produce`` keeps
    using the configured native executable or Cargo fallback.
    """
    token = _producer_override.set(callback)
    try:
        yield
    finally:
        _producer_override.reset(token)


def produce(request):
    """Run the shared Rust producer; never duplicate its calculations here.

    AUTOEQ_REPORT_PRODUCER can select a prebuilt report-producer executable.
    Source checkouts otherwise use Cargo's cached native build. Hosts that
    stage these scripts outside the checkout set AUTOEQ_WORKSPACE_ROOT to
    the real workspace; it defaults to this file's own checkout root.
    """
    callback = _producer_override.get()
    if callback is not None:
        json.dumps(request, ensure_ascii=False, allow_nan=False)
        result = callback(request)
        if not isinstance(result, str):
            raise TypeError("Shared AutoEQ report producer callback must return a string")
        return result

    executable = os.environ.get("AUTOEQ_REPORT_PRODUCER") or shutil.which("report-producer")
    if executable:
        command = [executable]
    else:
        workspace = os.environ.get("AUTOEQ_WORKSPACE_ROOT")
        root = Path(workspace) if workspace else Path(__file__).resolve().parents[2]
        cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
        command = [cargo, "run", "--quiet", "--manifest-path", str(root / "Cargo.toml"),
                   "-p", "autoeq-report-wasm", "--bin", "report-producer", "--"]
    serialized_request = json.dumps(request, ensure_ascii=False, allow_nan=False)
    result = subprocess.run(command, input=serialized_request,
                            text=True, capture_output=True, check=False, timeout=120)
    if result.returncode:
        raise RuntimeError("Shared AutoEQ report producer failed: " + result.stderr.strip())
    return result.stdout
