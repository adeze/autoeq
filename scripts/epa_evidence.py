"""Validate portable level-matched EPA receipts without application side effects."""

import math


def inspect_level_matched_epa(metadata):
    """Check declared level-match receipts; legacy absence is not a comparison."""
    if not isinstance(metadata, dict):
        raise ValueError("invalid level-matched EPA evidence")
    def checked(metrics):
        if metrics is not None and not isinstance(metrics, dict):
            raise ValueError("invalid level-matched EPA evidence")
        evidence = (metrics or {}).get("level_matched")
        if evidence is None:
            return None
        if not isinstance(evidence, dict):
            raise ValueError("invalid level-matched EPA evidence")
        if any(not isinstance(evidence.get(side), dict) for side in ("pre", "post")):
            raise ValueError("invalid level-matched EPA evidence")
        digest = evidence.get("frequency_grid_sha256", "")
        preferences = [(evidence.get(side) or {}).get("preference") for side in ("pre", "post")]
        values = [*preferences, evidence.get("preference_delta"), evidence.get("pre_reference_db"),
                  evidence.get("post_reference_db"), evidence.get("listening_level_phon")]
        samples = evidence.get("sample_count")
        if (evidence.get("method") != "log_frequency_pressure_energy_1_2_khz_v1"
                or evidence.get("reference_band_hz") != [1000.0, 2000.0]
                or not isinstance(samples, int) or isinstance(samples, bool) or samples < 2
                or not isinstance(digest, str) or len(digest) != 64
                or any(character not in "0123456789abcdef" for character in digest)
                or any(not isinstance(value, (int, float)) or isinstance(value, bool)
                       or not math.isfinite(value) for value in values)
                or not math.isclose(preferences[1] - preferences[0], evidence["preference_delta"],
                                    rel_tol=1e-9, abs_tol=1e-9)):
            raise ValueError("invalid level-matched EPA evidence")
        return {key: evidence[key] for key in (
            "method", "reference_band_hz", "frequency_grid_sha256", "sample_count",
            "pre_reference_db", "post_reference_db", "listening_level_phon", "preference_delta")}

    channel_metrics = metadata.get("epa_per_channel")
    if channel_metrics is not None and not isinstance(channel_metrics, dict):
        raise ValueError("invalid level-matched EPA evidence")
    channels = {name: checked(metrics) for name, metrics in sorted((channel_metrics or {}).items())}
    return {"per_channel": channels, "multichannel": checked(metadata.get("epa_multichannel")),
            "scope": "spectral_diagnostic_not_equal_loudness_or_measured_listener_preference"}



def inspect_epa_spectral_flatness(metadata):
    """Validate frozen EPA loss receipts; this does not rescore raw captures."""
    import json
    error = "invalid EPA spectral-flatness evidence"
    if not isinstance(metadata, dict):
        raise ValueError(error)
    modern = any(
        stage.get("stage") == "delivered_epa_non_regression"
        and "criterion=configured_frozen_epa_spectral_flatness_v1" in stage.get("advisories", [])
        for stage in metadata.get("stage_outcomes", []) if isinstance(stage, dict)
    )
    report = metadata.get("correction_acceptance") or {}
    quality = report.get("acoustic_quality") or {}
    rows = []
    present = modern
    finite = lambda value: isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
    for partition in ("training", "held_out"):
        metrics = quality.get(partition) or {}
        comparison = metrics.get("configured_baseline") or {}
        for residual in comparison.get("residuals", []):
            check = residual.get("epa_spectral_flatness")
            row = {"partition": partition, "logical_input": residual.get("score", {}).get("logical_input"),
                   "seat_index": residual.get("seat_index"), "assessment": None}
            rows.append(row)
            if check is None:
                continue
            present = True
            if not isinstance(check, dict) or not isinstance(check.get("passed"), bool):
                raise ValueError(error)
            if check.get("id") == "epa_spectral_flatness_unavailable":
                if check["passed"] or check.get("observed") is not None or check.get("limit") is not None:
                    raise ValueError(error)
                continue
            try:
                detail = json.loads(check.get("diagnostic", ""))
            except (TypeError, ValueError):
                raise ValueError(error) from None
            if not isinstance(detail, dict):
                raise ValueError(error)
            before, after = detail.get("pre_loss_db"), detail.get("post_loss_db")
            delta, limit = check.get("observed"), check.get("limit")
            margin = detail.get("signed_violation_margin")
            if (check.get("id") != "epa_spectral_flatness" or check.get("kind") != "quality"
                    or detail.get("method") != "configured_frozen_epa_spectral_flatness_v1"
                    or detail.get("scope") != "target_spectral_error_not_measured_listener_preference"
                    or detail.get("comparison") != "at_most" or detail.get("unit") != "dB"
                    or not all(finite(value) for value in (before, after, delta, limit, margin))
                    or before < 0 or after < 0 or limit != 1e-9
                    or not math.isclose(delta, after-before, rel_tol=1e-12, abs_tol=1e-12)
                    or not math.isclose(margin, delta-limit, rel_tol=1e-12, abs_tol=1e-12)
                    or check["passed"] != (delta <= limit)):
                raise ValueError(error)
            score = residual.get("score") or {}
            digests = [score.get(key) for key in (
                "frequency_grid_sha256", "baseline_samples_sha256", "target_samples_sha256")]
            samples, band = score.get("sample_count"), score.get("evaluated_band_hz")
            if (any(not isinstance(value, str) or len(value) != 64
                    or any(char not in "0123456789abcdef" for char in value) for value in digests)
                    or not isinstance(samples, int) or isinstance(samples, bool) or samples < 2
                    or not isinstance(band, list) or len(band) != 2
                    or not all(finite(value) for value in band) or not 0 < band[0] < band[1]
                    or not isinstance(score.get("normalized_to_pre"), bool)
                    or not all(finite(score.get(key)) for key in ("level_offset_db", "post_reference_gain_db"))
                    or score.get("post_covers_baseline_grid") is not True):
                raise ValueError(error)
            mixes = [detail.get("erb_weight"), detail.get("band_weight")]
            bands = detail.get("band_weights") or {}
            band_weights = [bands.get(key) for key in ("bass_weight", "mid_weight", "treble_weight")]
            if (not all(finite(value) and value >= 0 for value in mixes + band_weights)
                    or not math.isfinite(sum(mixes)) or sum(mixes) <= 0
                    or (mixes[0] == 0 and sum(band_weights) <= 0)):
                raise ValueError(error)
            for key in ("frequency_grid_sha256", "baseline_samples_sha256", "target_samples_sha256",
                        "sample_count", "evaluated_band_hz", "normalized_to_pre", "level_offset_db",
                        "post_reference_gain_db"):
                if key not in score or detail.get(key) != score[key]:
                    raise ValueError(error)
            row["assessment"] = {"pre_loss_db": before, "post_loss_db": after,
                                 "delta_db": delta, "passed": check["passed"]}
    if not present:
        return None
    if (modern and (report.get("accepted") is True or report.get("outcome") == "accepted")
            and (not rows or any(row["assessment"] is None or not row["assessment"]["passed"] for row in rows))):
        raise ValueError("accepted graph lacks complete non-regressing EPA spectral-flatness evidence")
    return {"method": "configured_frozen_epa_spectral_flatness_v1", "criterion_applied": modern,
            "rows": rows, "scope": "target_spectral_error_not_measured_listener_preference"}
