"""Functions for extracting data from roomeq output JSON."""

from .native_report_data import helper as _native_helper


# Channel order mapping for sorting (classical order)
CHANNEL_ORDER_MAP = {
    "L": 10, "LEFT": 10,
    "R": 20, "RIGHT": 20,
    "C": 30, "CENTER": 30,
    "LFE": 40, "SUB": 40, "SUBWOOFER": 40, "LFE1": 41, "LFE2": 42,
    "SL": 50, "SURROUND LEFT": 50, "LS": 50,
    "SR": 60, "SURROUND RIGHT": 60, "RS": 60,
    "SBL": 70, "SURROUND BACK LEFT": 70, "LBS": 70, "LB": 70,
    "SBR": 80, "SURROUND BACK RIGHT": 80, "RBS": 80, "RB": 80,
    "FHL": 90, "FRONT HEIGHT LEFT": 90,
    "FHR": 100, "FRONT HEIGHT RIGHT": 100,
    "BHL": 110, "BACK HEIGHT LEFT": 110,
    "BHR": 120, "BACK HEIGHT RIGHT": 120,
}


def get_channel_sort_key(channel_name: str) -> tuple[int, str]:
    """Get sort key for a channel name."""
    name_upper = channel_name.upper()
    # Try exact match
    if name_upper in CHANNEL_ORDER_MAP:
        return (CHANNEL_ORDER_MAP[name_upper], channel_name)

    # Try to see if it starts with one of the keys (e.g. "L (tweeter)")
    for key, order in CHANNEL_ORDER_MAP.items():
        if name_upper.startswith(key) and (len(name_upper) == len(key) or not name_upper[len(key)].isalnum()):
            return (order, channel_name)

    # Default: large number to put unknown at the end
    return (1000, channel_name)


def compute_y_range(curves):
    """Return the canonical fifty-decibel display range."""
    return tuple(_native_helper("compute_y_range", curves=curves))


def compute_average_spl_in_range(curve, min_freq=20.0, max_freq=1200.0):
    """Delegate display-band averaging to the shared report producer."""
    return _native_helper("average_spl", curve=curve, min_hz=min_freq, max_hz=max_freq)


def extract_crossover_frequencies(channel_data):
    """Return sorted configured crossover and main band-split frequencies."""
    return _native_helper("crossover_frequencies", channel=channel_data)


def clip_curve_to_measured_band(curve, driver):
    """Clip matching arrays to the serialized measured band through shared code."""
    clipped = _native_helper("clip_curve", curve=curve, driver=driver)
    return curve if clipped == curve else clipped


def get_plottable_drivers(channel_data: dict | None) -> list[dict]:
    """Return driver dicts that carry their own measured initial curve.

    A channel with such drivers (multi-sub LFE, multi-driver speakers) is
    displayed per driver: Row 1 of the overview already expands them, and
    the EQ row plus the per-channel tabs follow the same rule so a
    two-subwoofer setup shows one EQ and one tab per subwoofer instead of
    a single collapsed LFE entry.
    """
    if not isinstance(channel_data, dict):
        return []
    drivers = channel_data.get("drivers") or []
    plottable = []
    for driver in drivers:
        if not isinstance(driver, dict):
            continue
        initial = driver.get("initial_curve") or {}
        if initial.get("freq") and initial.get("spl"):
            plottable.append(driver)
    return plottable


def _configured_sub_names(data: dict, channel_name: str, driver_count: int) -> list[str]:
    """Resolve friendly per-driver names from the effective config.

    For a bass-managed channel the logical channel (e.g. ``LFE``) maps to
    a speaker group (e.g. ``subs``) via ``system.speakers``, and the group
    lists its physical subwoofers by name (e.g. ``Left Sub``). Returns []
    when the config is absent or does not match ``driver_count`` so callers
    fall back to the serialized driver names.
    """
    if driver_count <= 0:
        return []
    effective = ((data.get("metadata") or {}).get("effective_config") or {})
    if not isinstance(effective, dict):
        return []
    system_speakers = ((effective.get("system") or {}).get("speakers") or {})
    speaker_key = system_speakers.get(channel_name) if isinstance(system_speakers, dict) else None
    speakers = effective.get("speakers") or {}
    group = speakers.get(speaker_key) if isinstance(speakers, dict) else None
    subwoofers = (group or {}).get("subwoofers") or [] if isinstance(group, dict) else []
    names = [
        str(entry.get("name"))
        for entry in subwoofers
        if isinstance(entry, dict) and entry.get("name")
    ]
    return names if len(names) == driver_count else []


def driver_display_names(data: dict, channel_name: str) -> list[str]:
    """Return one display name per plottable driver of ``channel_name``."""
    channel = (data.get("channels") or {}).get(channel_name) or {}
    drivers = get_plottable_drivers(channel)
    friendly = _configured_sub_names(data, channel_name, len(drivers))
    if friendly:
        return friendly
    names = []
    for index, driver in enumerate(drivers):
        raw = driver.get("name") or driver.get("index", index)
        names.append(str(raw))
    return names


def channel_has_eq(channel_data):
    """Detect serialized channel/driver EQ through the canonical report helper."""
    return _native_helper("channel_has_eq", channel=channel_data)


def display_channel_entries(data: dict) -> list[dict]:
    """Return canonical ordered channel and plottable-driver tab entries."""
    return _native_helper("channel_display_entries", data=data)


def driver_display_entries(data: dict) -> list[dict]:
    """Return tab entries with stable raw driver positions and identities."""
    return _native_helper("driver_display_entries", data=data)


def extract_eq_passes(channel_data: dict) -> list[dict]:
    """
    Extract EQ plugins from a channel, grouped by pass label.

    The 3-pass pipeline labels EQ plugins as:
    - "cea2034_speaker_correction" (Pass 1)
    - "room_eq_correction" (Pass 2, or unlabeled)
    - "user_preference" (Pass 3)

    Unlabeled EQ plugins are assigned to "Room EQ".

    Returns:
        List of dicts with keys: label, display_name, filters, color
    """
    plugins = channel_data.get("plugins", [])

    PASS_DISPLAY_NAMES = {
        "cea2034_speaker_correction": "Pass 1: Speaker Correction (CEA2034)",
        "room_eq_correction": "Pass 2: Room EQ",
        "user_preference": "Pass 3: User Preference",
    }

    PASS_COLORS = {
        "cea2034_speaker_correction": "rgba(255, 165, 0, 0.9)",   # orange
        "room_eq_correction": "rgba(100, 100, 255, 0.9)",         # blue
        "user_preference": "rgba(180, 100, 255, 0.9)",            # purple
    }

    passes: list[dict] = []

    for plugin in plugins:
        if plugin.get("plugin_type") != "eq":
            continue

        params = plugin.get("parameters", {})
        label = params.get("label", "")
        filters = params.get("filters", [])

        if not filters:
            continue

        display_name = PASS_DISPLAY_NAMES.get(label, "Room EQ")
        color = PASS_COLORS.get(label, "rgba(100, 100, 255, 0.9)")

        passes.append({
            "label": label,
            "display_name": display_name,
            "filters": filters,
            "color": color,
        })

    return passes


def get_all_crossover_frequencies(data: dict) -> list[float]:
    """
    Extract all unique crossover frequencies from all channels.

    Returns:
        Sorted list of unique crossover frequencies in Hz
    """
    all_freqs = set()
    channels = data.get("channels", {})

    for channel_data in channels.values():
        freqs = extract_crossover_frequencies(channel_data)
        all_freqs.update(freqs)

    return sorted(all_freqs)
