//! Rounds every f64 leaf in a serde_json Value to N significant digits, in
//! place. Integer Numbers are left untouched so that counters stay integers.

use serde_json::{Number, Value};

pub fn round_floats_in_place(value: &mut Value, sig_digits: u32, skip_keys: &[&str]) {
    match value {
        Value::Number(n) if n.is_f64() => {
            if let Some(f) = n.as_f64() {
                let rounded = round_sig(f, sig_digits);
                if let Some(num) = Number::from_f64(rounded) {
                    *n = num;
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                round_floats_in_place(v, sig_digits, skip_keys);
            }
        }
        Value::Object(obj) => {
            for (k, v) in obj.iter_mut() {
                if skip_keys.contains(&k.as_str()) {
                    continue;
                }
                round_floats_in_place(v, sig_digits, skip_keys);
            }
        }
        _ => {}
    }
}

fn round_sig(x: f64, sig: u32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let d = x.abs().log10().floor() as i32 + 1;
    let factor = 10f64.powi(sig as i32 - d);
    (x * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() <= b.abs() * 1e-12 + 1e-12
    }

    #[test]
    fn rounds_mid_magnitude() {
        assert!(approx(round_sig(12.34567, 5), 12.346));
    }

    #[test]
    fn rounds_small_magnitude() {
        assert!(approx(round_sig(0.00012345678, 5), 0.00012346));
    }

    #[test]
    fn rounds_large_magnitude() {
        assert!(approx(round_sig(12345.6789, 5), 12346.0));
    }

    #[test]
    fn rolls_over_on_round_up() {
        assert!(approx(round_sig(9.9999999, 5), 10.0));
    }

    #[test]
    fn handles_zero_and_negatives() {
        assert_eq!(round_sig(0.0, 5), 0.0);
        assert!(approx(round_sig(-12.34567, 5), -12.346));
    }

    #[test]
    fn leaves_nan_and_inf_alone() {
        assert!(round_sig(f64::NAN, 5).is_nan());
        assert!(round_sig(f64::INFINITY, 5).is_infinite());
        assert!(round_sig(f64::NEG_INFINITY, 5).is_infinite());
    }

    #[test]
    fn integer_numbers_remain_integers() {
        let mut v = json!({ "a": 5_u64, "b": 1.234567_f64, "c": -3_i64 });
        round_floats_in_place(&mut v, 5, &[]);
        assert!(v["a"].is_u64(), "a should remain u64, got {:?}", v["a"]);
        assert!(v["c"].is_i64(), "c should remain i64, got {:?}", v["c"]);
        assert!(approx(v["b"].as_f64().unwrap(), 1.2346));
    }

    #[test]
    fn walks_nested_objects_and_arrays() {
        let mut v = json!({
            "outer": {
                "vals": [1.234567_f64, 2.345678_f64, 99_u64],
                "inner": { "x": 0.00012345678_f64 }
            }
        });
        round_floats_in_place(&mut v, 5, &[]);
        assert!(approx(v["outer"]["vals"][0].as_f64().unwrap(), 1.2346));
        assert!(approx(v["outer"]["vals"][1].as_f64().unwrap(), 2.3457));
        assert!(v["outer"]["vals"][2].is_u64());
        assert!(approx(
            v["outer"]["inner"]["x"].as_f64().unwrap(),
            0.00012346
        ));
    }

    #[test]
    fn skips_excluded_keys() {
        let mut v = json!({
            "Gnss": {
                "GnssLatitude": 52.123456789_f64,
                "GnssLongitude": 4.987654321_f64,
                "Speed": 1.234567_f64
            },
            "Other": 1.234567_f64
        });
        round_floats_in_place(&mut v, 5, &["GnssLatitude", "GnssLongitude"]);
        assert_eq!(v["Gnss"]["GnssLatitude"].as_f64().unwrap(), 52.123456789);
        assert_eq!(v["Gnss"]["GnssLongitude"].as_f64().unwrap(), 4.987654321);
        assert!(approx(v["Gnss"]["Speed"].as_f64().unwrap(), 1.2346));
        assert!(approx(v["Other"].as_f64().unwrap(), 1.2346));
    }
}
