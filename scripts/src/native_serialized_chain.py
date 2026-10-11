"""Thin bridge for canonical serialized RoomEQ plugin-chain responses."""

import json
import math

from .native_report import produce


_PRIVATE_FIR_FIELDS = {
    "_fir_sample_rate",
    "_fir_taps",
    "_native_fir_resource_status",
    "_native_fir_resource_reason",
    "_native_fir_sample_rate",
    "_native_fir_resource_key",
    "_native_fir_graph_sha256",
    "_native_fir_expected_resource_sha256",
}


def _legacy_plugin(plugin):
    """Copy a saved wrapper and normalize only historical absent defaults."""
    if not isinstance(plugin, dict):
        raise ValueError("serialized plugin must be an object")
    plugin_type = plugin.get("plugin_type")
    if not isinstance(plugin_type, str) or not plugin_type:
        raise ValueError("serialized plugin requires a non-empty plugin_type")
    raw_parameters = plugin.get("parameters", {})
    if raw_parameters is None:
        raw_parameters = {}
    if not isinstance(raw_parameters, dict):
        raise ValueError(f"serialized {plugin_type} parameters must be an object")

    # Do not serialize loader/host caches. Their contents are consumed below
    # solely to resolve the exact captured FIR response through native_fir_response.
    parameters = {key: value for key, value in raw_parameters.items()
                  if key not in _PRIVATE_FIR_FIELDS}
    normalized_type = plugin_type.lower()
    if normalized_type == "gain":
        parameters.setdefault("gain_db", 0.0)
    elif normalized_type == "delay":
        parameters.setdefault("delay_ms", 0.0)
    elif normalized_type == "crossover":
        parameters.setdefault("type", "LR24")
        parameters.setdefault("frequency", 80.0)
        parameters.setdefault("output", "low")
    elif normalized_type == "eq":
        parameters.setdefault("filters", [])
    return {"plugin_type": normalized_type, "parameters": parameters}, raw_parameters


def _fir_response_for(plugin_parameters, frequencies, sample_rate):
    """Resolve FIR evidence without paths, then return same-grid complex pairs."""
    from .native_fir_response import fir_response

    ir_file = plugin_parameters.get("ir_file")
    if not isinstance(ir_file, str) or not ir_file:
        raise ValueError("Convolution plugin requires an ir_file")

    native_status = plugin_parameters.get("_native_fir_resource_status")
    if native_status is not None:
        if native_status != "available":
            reason = plugin_parameters.get("_native_fir_resource_reason")
            detail = f": {reason}" if isinstance(reason, str) and reason else ""
            raise ValueError(f"Convolution FIR resource is unavailable{detail}")
        resource_key = plugin_parameters.get("_native_fir_resource_key")
        if resource_key != ir_file:
            raise ValueError("Physical FIR resource key does not match the graph ir_file")
        if plugin_parameters.get("_native_fir_sample_rate") != sample_rate:
            raise ValueError("Physical FIR sample rate does not match the report sample rate")
        response = fir_response(
            frequencies,
            sample_rate,
            resource_key=resource_key,
            graph_sha256=plugin_parameters.get("_native_fir_graph_sha256"),
            resource_sha256=plugin_parameters.get("_native_fir_expected_resource_sha256"),
        )
    else:
        taps = plugin_parameters.get("_fir_taps")
        if not taps:
            raise ValueError(
                f"Convolution FIR resource {ir_file!r} is unavailable: no captured native "
                "resource or verified standalone FIR cache is available"
            )
        if plugin_parameters.get("_fir_sample_rate") != sample_rate:
            raise ValueError("Physical FIR sample rate does not match the report sample rate")
        response = fir_response(frequencies, sample_rate, taps=taps)
    return [[value.real, value.imag] for value in response]


def serialized_plugin_chain_response(plugins, frequencies, sample_rate):
    """Evaluate one validated serialized plugin chain on its supplied grid.

    The grid is kept in caller order. Convolution taps or resource identities
    are resolved through the existing frozen-FIR adapter; no filesystem lookup
    occurs here.
    """
    if not isinstance(plugins, list):
        raise ValueError("serialized plugins must be a list")
    if isinstance(sample_rate, bool) or not isinstance(sample_rate, (int, float)):
        raise ValueError("DSP sample rate must be numeric")
    sample_rate = float(sample_rate)
    if not math.isfinite(sample_rate) or sample_rate <= 0:
        raise ValueError("DSP sample rate must be finite and positive")
    if not isinstance(frequencies, list) or not frequencies or len(frequencies) > 100_000:
        raise ValueError("serialized plugin response requires a bounded non-empty frequency grid")
    for frequency in frequencies:
        if (isinstance(frequency, bool) or not isinstance(frequency, (int, float))
                or not math.isfinite(frequency) or frequency < 0
                or frequency > sample_rate / 2):
            raise ValueError("plugin response frequencies must be finite and between DC and Nyquist")

    normalized = []
    fir_responses = {}
    for plugin in plugins:
        wrapper, raw_parameters = _legacy_plugin(plugin)
        if wrapper["plugin_type"] == "convolution":
            ir_file = wrapper["parameters"].get("ir_file")
            response = _fir_response_for(raw_parameters, frequencies, sample_rate)
            previous = fir_responses.get(ir_file)
            if previous is not None and previous != response:
                raise ValueError(f"Convolution resource {ir_file!r} resolved inconsistently")
            fir_responses[ir_file] = response
        normalized.append(wrapper)

    request = {
        "operation": "serialized_plugin_chain_response",
        "sample_rate": sample_rate,
        "frequencies": frequencies,
        "plugins": normalized,
    }
    if fir_responses:
        request["fir_responses"] = fir_responses
    try:
        pairs = json.loads(produce(request))
    except (RuntimeError, json.JSONDecodeError) as error:
        raise ValueError(f"Canonical serialized plugin-chain response unavailable: {error}") from error
    if not isinstance(pairs, list) or len(pairs) != len(frequencies):
        raise ValueError("Canonical serialized plugin-chain response returned an invalid grid")
    result = []
    for pair in pairs:
        if (not isinstance(pair, list) or len(pair) != 2
                or any(isinstance(value, bool) or not isinstance(value, (int, float))
                       or not math.isfinite(value) for value in pair)):
            raise ValueError("Canonical serialized plugin-chain response returned an invalid complex value")
        result.append(complex(pair[0], pair[1]))
    return result
