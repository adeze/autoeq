//! Canonical limiter contract shared with portable serialized DSP.
#[doc(inline)]
#[doc(inline)]
pub use autoeq_core::runtime_limiter::{
    LABEL, LOOKAHEAD_MS, RELEASE_MS, ceiling, latency_samples, plugin,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_is_explicit_and_fail_closed() {
        assert_eq!(ceiling(&plugin(0.0)).unwrap(), -1.0);
        assert_eq!(ceiling(&plugin(-3.0)).unwrap(), -3.0);
        for (key, value) in [
            ("mix", serde_json::json!(0.5)),
            ("soft", serde_json::json!(true)),
            ("lookahead_ms", serde_json::json!(0.0)),
            ("threshold_db", serde_json::json!(1.0)),
            ("room_eq_stage", serde_json::json!("pre_route")),
        ] {
            let mut modified = plugin(0.0);
            modified.parameters[key] = value;
            assert!(ceiling(&modified).is_err(), "{key}");
        }
        assert_eq!(latency_samples(48_000.0), 240);
        assert_eq!(latency_samples(44_100.0), 220);
    }
}
