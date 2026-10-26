//! Bounds display payloads without resampling the underlying acoustic evidence.

// Rust guideline compliant 2026-02-21

use serde_json::Value;

// A 2048-pixel plot needs at most two extrema per pixel. The budget also
// includes endpoints and gap markers so missing samples cannot become lines.
const MAX_DISPLAY_POINTS: usize = 4096;

/// Select original sample pairs, retaining extrema, endpoints, and missing-data gaps.
pub(crate) fn display_samples(x: &Value, y: &Value) -> (Value, Value) {
    let (Some(xs), Some(ys)) = (x.as_array(), y.as_array()) else {
        return (x.clone(), y.clone());
    };
    let count = xs.len();
    if count <= MAX_DISPLAY_POINTS
        || count != ys.len()
        || xs.iter().any(|x| x.as_f64().is_none_or(|x| !x.is_finite()))
    {
        return (x.clone(), y.clone());
    }
    // Each bucket contributes two extrema and up to two intervening gaps.
    // Keeping endpoints separately leaves the total within the display cap.
    let buckets = (MAX_DISPLAY_POINTS - 2) / 4;
    let mut indices = Vec::with_capacity(buckets * 2 + 2);
    indices.push(0);
    for bucket in 0..buckets {
        let start = bucket * count / buckets;
        let end = (bucket + 1) * count / buckets;
        let mut minimum: Option<(usize, f64)> = None;
        let mut maximum: Option<(usize, f64)> = None;
        for (index, value) in ys.iter().enumerate().take(end).skip(start) {
            let Some(value) = value.as_f64().filter(|value| value.is_finite()) else {
                continue;
            };
            if minimum.is_none_or(|(_, previous)| value < previous) {
                minimum = Some((index, value));
            }
            if maximum.is_none_or(|(_, previous)| value > previous) {
                maximum = Some((index, value));
            }
        }
        if let Some((index, _)) = minimum {
            indices.push(index);
        }
        if let Some((index, _)) = maximum {
            indices.push(index);
        }
    }
    indices.push(count - 1);
    indices.sort_unstable();
    indices.dedup();
    let mut display_x = Vec::with_capacity(MAX_DISPLAY_POINTS);
    let mut display_y = Vec::with_capacity(MAX_DISPLAY_POINTS);
    let mut previous = None;
    for index in indices {
        if let Some(previous) = previous
            && let Some(gap) = (previous + 1..index)
                .find(|&gap| ys[gap].as_f64().is_none_or(|value| !value.is_finite()))
        {
            display_x.push(xs[gap].clone());
            display_y.push(Value::Null);
        }
        display_x.push(xs[index].clone());
        display_y.push(ys[index].clone());
        previous = Some(index);
    }
    debug_assert!(display_x.len() <= MAX_DISPLAY_POINTS);
    (Value::Array(display_x), Value::Array(display_y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn long_ir_keeps_exact_peak_times_pre_ringing_and_gaps_within_the_cap() {
        let count = 131_072;
        let xs: Vec<_> = (0..count).map(|index| index as f64 / 48.0 - 50.0).collect();
        let mut ys = vec![json!(0.0); count];
        ys[117] = json!(-0.1);
        ys[48_123] = json!(1.0);
        ys[48_500] = Value::Null;
        ys[48_777] = json!(-0.4);
        let x = json!(xs);
        let y = json!(ys);
        let (display_x, display_y) = display_samples(&x, &y);
        let dx = display_x.as_array().unwrap();
        let dy = display_y.as_array().unwrap();
        assert!(dx.len() <= MAX_DISPLAY_POINTS);
        assert_eq!(dx.len(), dy.len());
        assert_eq!((dx.first(), dy.first()), (Some(&x[0]), Some(&y[0])));
        assert_eq!(dx.last(), Some(&x[count - 1]));
        for index in [117, 48_123, 48_777] {
            assert!(
                dx.iter()
                    .zip(dy)
                    .any(|(time, value)| time == &x[index] && value == &y[index])
            );
        }
        let positive = dy.iter().position(|value| value == &json!(1.0)).unwrap();
        let negative = dy.iter().position(|value| value == &json!(-0.4)).unwrap();
        assert!(dy[positive..negative].iter().any(Value::is_null));
        assert!(
            dx.windows(2)
                .all(|pair| pair[0].as_f64() < pair[1].as_f64())
        );
        assert_eq!(y[48_500], Value::Null, "source evidence remains untouched");
    }

    #[test]
    fn short_or_mismatched_series_remain_unchanged() {
        let x = json!([-1.0, 0.0, 1.0]);
        let y = json!([0.1, 1.0, 0.0]);
        assert_eq!(display_samples(&x, &y), (x.clone(), y.clone()));
        let long = json!(vec![0.0; 10_000]);
        assert_eq!(display_samples(&long, &y), (long.clone(), y));
    }
}
