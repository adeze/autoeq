"""Consume native coherent responses without reconstructing routing in Python."""

from .native_driver_replay import validate_native_curve


def resolve_native_source_curves(data):
    """Return handled/curves and attach explicit per-source unavailable reasons."""
    if not getattr(data, "native_dsp_replay_present", False):
        return False, {}
    replay = getattr(data, "native_dsp_replay", None)
    table = replay.get("routed_sources") if isinstance(replay, dict) else None
    graph = ((data.get("metadata") or {}).get("bass_management") or {}).get("routing_graph") or {}
    expected = graph.get("input_channels") or list((data.get("deployed_source_curves") or {}).keys())
    if not expected:
        expected = list((data.get("channels") or {}).keys())
    expected = [name for name in expected if isinstance(name, str) and name]
    unavailable = {}
    curves = {}
    if not isinstance(table, dict):
        for source in expected or ["Report"]:
            unavailable[source] = "Native routed-source replay is missing or malformed"
    else:
        for source in sorted(set(expected) | set(table), key=str):
            if not isinstance(source, str) or not source:
                unavailable["Report"] = "Native routed-source identity is malformed"
                continue
            state = table.get(source)
            if not isinstance(state, dict):
                unavailable[source] = "No native routed-source result exists for this input"
            elif state.get("status") == "unavailable":
                reason = state.get("reason")
                unavailable[source] = reason if isinstance(reason, str) and reason else "Native routed-source response is unavailable"
            elif state.get("status") == "available":
                curve = state.get("curve")
                valid, reason = validate_native_curve(curve)
                if valid:
                    curves[source] = curve
                else:
                    unavailable[source] = reason or "Native routed-source curve is invalid"
            else:
                unavailable[source] = "Native routed-source state is unsupported"
    data.native_routed_source_unavailability = unavailable
    return True, curves
