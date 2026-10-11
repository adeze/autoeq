"""File-backed transport for a host-injected native report producer."""

from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import threading
import time
from typing import Any

MAX_MESSAGE_BYTES = 64 * 1024 * 1024
_POLL_INTERVAL_SECONDS = 0.01


def build_host_report_payload(input_path, output_path, sections_path, directory):
    """Retain the canonical loaded object while the host computes shared sections."""
    from .loaders import load_roomeq_json

    data = load_roomeq_json(Path(input_path))
    try:
        return build_host_report_payload_from_data(
            data, output_path, sections_path, directory, input_path=Path(input_path)
        )
    finally:
        data.close()


def build_host_report_payload_from_data(
    data, output_path, sections_path, directory, input_path=None
):
    """Build a report from an already loaded RoomEqData without reopening inputs."""
    from .native_report import using_producer
    from .report import build_report_payload

    transport = file_producer(directory)
    cache = {}
    cache_bytes = 0

    def cached_producer(request):
        nonlocal cache_bytes
        key = json.dumps(request, ensure_ascii=False, allow_nan=False, sort_keys=True)
        if key in cache:
            return cache[key]
        output = transport(request)
        size = len(key.encode("utf-8")) + len(output.encode("utf-8"))
        if cache_bytes + size <= MAX_MESSAGE_BYTES:
            cache[key] = output
            cache_bytes += size
        return output

    with using_producer(cached_producer):
        payload = build_report_payload(
            data,
            output_json_path=Path(input_path).resolve() if input_path is not None else None,
            shared_sections=json.loads(Path(sections_path).read_text(encoding="utf-8")),
        )
    Path(output_path).write_text(
        json.dumps(payload, ensure_ascii=False, allow_nan=False), encoding="utf-8"
    )
    return payload


def file_producer(directory: Path, timeout_s: float = 60.0):
    """Create a serialized request callback using host-owned JSON files.

    Each call atomically publishes ``request.json`` with an increasing request
    ID and waits for the host to atomically publish ``response.json``. The
    host must echo that ID. The callback performs no subprocess or network I/O.
    """
    directory = Path(directory)
    request_path = directory / "request.json"
    response_path = directory / "response.json"
    lock = threading.Lock()
    next_id = 1

    def produce(request: Any) -> str:
        nonlocal next_id
        with lock:
            request_id = next_id
            next_id += 1

            # Remove an abandoned response before publishing another ID.
            response_path.unlink(missing_ok=True)
            envelope = {"id": request_id, "request": request}
            try:
                encoded = json.dumps(
                    envelope,
                    ensure_ascii=False,
                    allow_nan=False,
                    separators=(",", ":"),
                ).encode("utf-8")
            except (TypeError, ValueError) as error:
                raise ValueError(
                    f"Report request {request_id} is not strict JSON: {error}"
                ) from error
            _check_size(len(encoded), "request", request_id)

            temporary_path: Path | None = None
            try:
                descriptor, temporary_name = tempfile.mkstemp(
                    prefix=f"request-{request_id}-", suffix=".tmp", dir=directory
                )
                temporary_path = Path(temporary_name)
                with os.fdopen(descriptor, "wb") as temporary_file:
                    temporary_file.write(encoded)
                    temporary_file.flush()
                    os.fsync(temporary_file.fileno())
                os.replace(temporary_path, request_path)
                temporary_path = None
            except OSError as error:
                raise RuntimeError(
                    f"Could not publish report request {request_id} in {directory}: {error}"
                ) from error
            finally:
                if temporary_path is not None:
                    temporary_path.unlink(missing_ok=True)

            deadline = time.monotonic() + timeout_s
            while True:
                if response_path.exists():
                    try:
                        size = response_path.stat().st_size
                        _check_size(size, "response", request_id)
                        response_bytes = response_path.read_bytes()
                    except FileNotFoundError:
                        # The host replaces response files atomically; retry if
                        # the previous path disappeared during its replacement.
                        continue
                    finally:
                        response_path.unlink(missing_ok=True)
                    return _decode_response(response_bytes, request_id)

                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    request_path.unlink(missing_ok=True)
                    raise TimeoutError(
                        f"Timed out after {timeout_s:g}s waiting for host report "
                        f"response to request {request_id} in {directory}"
                    )
                time.sleep(min(_POLL_INTERVAL_SECONDS, remaining))

    return produce


def _decode_response(response_bytes: bytes, request_id: int) -> str:
    try:
        response = json.loads(
            response_bytes.decode("utf-8"),
            parse_constant=_reject_nonfinite_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise RuntimeError(
            f"Host returned invalid JSON for report request {request_id}: {error}"
        ) from error
    if not isinstance(response, dict):
        raise RuntimeError(
            f"Host response for report request {request_id} must be a JSON object"
        )
    response_id = response.get("id")
    if type(response_id) is not int or response_id != request_id:
        raise RuntimeError(
            f"Host response ID mismatch for report request {request_id}: "
            f"received {response_id!r}"
        )
    ok = response.get("ok")
    if type(ok) is not bool:
        raise RuntimeError(
            f"Host response for report request {request_id} must contain a boolean `ok`"
        )
    if not ok:
        error = response.get("error")
        detail = error if isinstance(error, str) and error else "unspecified host error"
        raise RuntimeError(f"Host report request {request_id} failed: {detail}")
    output = response.get("output")
    if not isinstance(output, str):
        raise RuntimeError(
            f"Host response for report request {request_id} has no string `output`"
        )
    _check_size(len(output.encode("utf-8")), "output", request_id)
    return output


def _check_size(size: int, message_kind: str, request_id: int) -> None:
    if size > MAX_MESSAGE_BYTES:
        raise ValueError(
            f"Report {message_kind} for request {request_id} is {size} bytes; "
            f"limit is {MAX_MESSAGE_BYTES} bytes"
        )


def _reject_nonfinite_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON number {value} is not permitted")
