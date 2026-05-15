//! Cross-frame derived/computed values.
//!
//! Runs after all frames in a publish cycle have been merged into one JSON
//! object. Each rule reads one or more leaves via JSON Pointer; if any input
//! is missing (frame not received this cycle) the rule writes nothing.
//!
//! Output paths must also have a matching HaEntity in ha_discovery.rs;
//! the registry_matches_json_leaves test enforces this.

use serde_json::{Value, json};

pub fn apply_derived(merged: &mut Value) {
    apply_battery(merged);
    apply_vesc(merged);
    apply_throttle(merged);
    apply_mppt(merged);
    apply_gan_mppt(merged);
}

fn apply_throttle(merged: &mut Value) {
    let active = if merged.pointer("/Throttle/ToVescDutyCycle").is_some() {
        Some("DutyCycle")
    } else if merged.pointer("/Throttle/ToVescCurrent").is_some() {
        Some("Current")
    } else if merged.pointer("/Throttle/ToVescCurrentRelative").is_some() {
        Some("CurrentRelative")
    } else if merged.pointer("/Throttle/ToVescRpm").is_some() {
        Some("Rpm")
    } else {
        None
    };

    if let Some(s) = active {
        set_leaf(merged, &["Throttle", "ActiveControlType"], json!(s));
    }
}

fn apply_battery(merged: &mut Value) {
    let pack_voltage = read_f64(
        merged,
        "/EoiBattery/CellVoltages13_14PackAndStack/pack_voltage",
    );
    let stack_voltage = read_f64(
        merged,
        "/EoiBattery/CellVoltages13_14PackAndStack/stack_voltage",
    );
    let perri_current = read_f64(merged, "/EoiBattery/PackAndPerriCurrent/perri_current");
    let charge_current = read_f64(
        merged,
        "/EoiBattery/ChargeAndDischargeCurrent/charge_current",
    );
    let discharge_current = read_f64(
        merged,
        "/EoiBattery/ChargeAndDischargeCurrent/discharge_current",
    );

    // The BMS reports pack_current on 0x100 as charge + |discharge| + perri rather
    // than a true net current, so override it with the Kirchhoff sum of the
    // already-sign-corrected component currents when all three are available.
    // this is done because BMS reports it incorrectly
    let pack_current = match (charge_current, discharge_current, perri_current) {
        (Some(c), Some(d), Some(p)) => {
            let derived = c + d + p;
            set_leaf(
                merged,
                &["EoiBattery", "PackAndPerriCurrent", "pack_current"],
                json!(derived as f32),
            );
            Some(derived)
        }
        _ => read_f64(merged, "/EoiBattery/PackAndPerriCurrent/pack_current"),
    };

    if let (Some(v), Some(i)) = (pack_voltage, pack_current) {
        set_leaf(
            merged,
            &["EoiBattery", "PackAndPerriCurrent", "pack_power"],
            json!((v * i) as f32),
        );
    }
    if let (Some(v), Some(i)) = (pack_voltage, perri_current) {
        set_leaf(
            merged,
            &["EoiBattery", "PackAndPerriCurrent", "perri_power"],
            json!((v * i) as f32),
        );
    }
    if let (Some(v), Some(i)) = (pack_voltage, charge_current) {
        set_leaf(
            merged,
            &["EoiBattery", "ChargeAndDischargeCurrent", "charge_power"],
            json!((v * i) as f32),
        );
    }
    if let (Some(v), Some(i)) = (pack_voltage, discharge_current) {
        set_leaf(
            merged,
            &["EoiBattery", "ChargeAndDischargeCurrent", "discharge_power"],
            json!((v * i) as f32),
        );
    }
    if let (Some(v), Some(i)) = (stack_voltage, pack_current) {
        set_leaf(
            merged,
            &["EoiBattery", "CellVoltages13_14PackAndStack", "stack_power"],
            json!((v * i) as f32),
        );
    }
}

fn apply_vesc(merged: &mut Value) {
    let v = read_f64(merged, "/Vesc/StatusMessage5/input_voltage");
    let i = read_f64(merged, "/Vesc/StatusMessage4/total_input_current");
    if let (Some(v), Some(i)) = (v, i) {
        set_leaf(
            merged,
            &["Vesc", "StatusMessage4", "total_input_power"],
            json!((v * i) as f32),
        );
    }
}

fn apply_mppt(merged: &mut Value) {
    for node in 0..8u8 {
        for ch in 0..4u8 {
            let v = read_f64(
                merged,
                &format!("/Mppt/Id{node}/Channel{ch}/Power/voltage_in"),
            );
            let i = read_f64(
                merged,
                &format!("/Mppt/Id{node}/Channel{ch}/Power/current_in"),
            );
            if let (Some(v), Some(i)) = (v, i) {
                set_leaf(
                    merged,
                    &[
                        "Mppt",
                        &format!("Id{node}"),
                        &format!("Channel{ch}"),
                        "Power",
                        "power_in",
                    ],
                    json!((v * i) as f32),
                );
            }
        }

        let v = read_f64(merged, &format!("/Mppt/Id{node}/Power/voltage_out"));
        let i = read_f64(merged, &format!("/Mppt/Id{node}/Power/current_out"));
        if let (Some(v), Some(i)) = (v, i) {
            set_leaf(
                merged,
                &["Mppt", &format!("Id{node}"), "Power", "power_out"],
                json!((v * i) as f32),
            );
        }
    }
}

fn apply_gan_mppt(merged: &mut Value) {
    for node in 0..16u8 {
        let iv = read_f64(merged, &format!("/GanMppt/Id{node}/Power/input_voltage"));
        let ii = read_f64(merged, &format!("/GanMppt/Id{node}/Power/input_current"));
        let ov = read_f64(merged, &format!("/GanMppt/Id{node}/Power/output_voltage"));
        let oi = read_f64(merged, &format!("/GanMppt/Id{node}/Power/output_current"));

        let input_power = match (iv, ii) {
            (Some(v), Some(i)) => Some(v * i),
            _ => None,
        };
        let output_power = match (ov, oi) {
            (Some(v), Some(i)) => Some(v * i),
            _ => None,
        };

        if let Some(p) = input_power {
            set_leaf(
                merged,
                &["GanMppt", &format!("Id{node}"), "Power", "input_power"],
                json!(p as f32),
            );
        }
        if let Some(p) = output_power {
            set_leaf(
                merged,
                &["GanMppt", &format!("Id{node}"), "Power", "output_power"],
                json!(p as f32),
            );
        }
        if let (Some(ip), Some(op)) = (input_power, output_power) {
            if ip.abs() >= 1.0 {
                set_leaf(
                    merged,
                    &["GanMppt", &format!("Id{node}"), "Power", "efficiency"],
                    json!((op / ip * 100.0) as f32),
                );
            }
        }
    }
}

fn read_f64(root: &Value, pointer: &str) -> Option<f64> {
    root.pointer(pointer).and_then(Value::as_f64)
}

fn set_leaf(root: &mut Value, path: &[&str], value: Value) {
    let Some((last, prefix)) = path.split_last() else {
        return;
    };
    let mut cur = root;
    for seg in prefix {
        let Some(map) = cur.as_object_mut() else {
            return;
        };
        if !map.contains_key(*seg) {
            map.insert((*seg).to_string(), json!({}));
        }
        let next = map.get_mut(*seg).expect("just inserted");
        if !next.is_object() {
            return;
        }
        cur = next;
    }
    if let Some(map) = cur.as_object_mut() {
        map.insert((*last).to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_pack_power_when_both_inputs_present() {
        let mut root = json!({
            "EoiBattery": {
                "PackAndPerriCurrent": { "pack_current": 10.0, "perri_current": 0.5 },
                "CellVoltages13_14PackAndStack": { "pack_voltage": 50.0, "stack_voltage": 49.0 }
            }
        });
        apply_derived(&mut root);
        assert_eq!(
            root.pointer("/EoiBattery/PackAndPerriCurrent/pack_power")
                .and_then(Value::as_f64),
            Some(500.0)
        );
        assert_eq!(
            root.pointer("/EoiBattery/PackAndPerriCurrent/perri_power")
                .and_then(Value::as_f64),
            Some(25.0)
        );
        assert_eq!(
            root.pointer("/EoiBattery/CellVoltages13_14PackAndStack/stack_power")
                .and_then(Value::as_f64),
            Some(490.0)
        );
    }

    #[test]
    fn battery_pack_current_derived_from_kirchhoff() {
        let mut root = json!({
            "EoiBattery": {
                "PackAndPerriCurrent": { "pack_current": 99.0, "perri_current": -0.57 },
                "ChargeAndDischargeCurrent": { "charge_current": 8.06, "discharge_current": -15.97 },
                "CellVoltages13_14PackAndStack": { "pack_voltage": 50.0, "stack_voltage": 49.0 }
            }
        });
        apply_derived(&mut root);

        let pack_current = root
            .pointer("/EoiBattery/PackAndPerriCurrent/pack_current")
            .and_then(Value::as_f64)
            .expect("pack_current present");
        assert!(
            (pack_current - (8.06 + -15.97 + -0.57)).abs() < 1e-4,
            "got {pack_current}"
        );

        let pack_power = root
            .pointer("/EoiBattery/PackAndPerriCurrent/pack_power")
            .and_then(Value::as_f64)
            .expect("pack_power present");
        assert!(
            (pack_power - 50.0 * pack_current).abs() < 1e-3,
            "got {pack_power}"
        );
    }

    #[test]
    fn battery_pack_power_absent_when_voltage_missing() {
        let mut root = json!({
            "EoiBattery": {
                "PackAndPerriCurrent": { "pack_current": 10.0, "perri_current": 0.0 }
            }
        });
        apply_derived(&mut root);
        assert!(
            root.pointer("/EoiBattery/PackAndPerriCurrent/pack_power")
                .is_none()
        );
    }

    #[test]
    fn battery_pack_power_absent_when_current_missing() {
        let mut root = json!({
            "EoiBattery": {
                "CellVoltages13_14PackAndStack": { "pack_voltage": 50.0, "stack_voltage": 49.0 }
            }
        });
        apply_derived(&mut root);
        assert!(
            root.pointer("/EoiBattery/PackAndPerriCurrent/pack_power")
                .is_none()
        );
    }

    #[test]
    fn throttle_active_control_type_picks_present_command() {
        let cases = [
            (
                json!({ "Throttle": { "ToVescCurrentRelative": 0.0 } }),
                "CurrentRelative",
            ),
            (json!({ "Throttle": { "ToVescRpm": 1234.0 } }), "Rpm"),
            (json!({ "Throttle": { "ToVescCurrent": 5.0 } }), "Current"),
            (
                json!({ "Throttle": { "ToVescDutyCycle": 50.0 } }),
                "DutyCycle",
            ),
        ];
        for (mut root, expected) in cases {
            apply_derived(&mut root);
            assert_eq!(
                root.pointer("/Throttle/ActiveControlType")
                    .and_then(Value::as_str),
                Some(expected),
            );
        }
    }

    #[test]
    fn throttle_active_control_type_absent_when_no_command() {
        let mut root = json!({ "Throttle": { "Status": { "value": 0.0 } } });
        apply_derived(&mut root);
        assert!(root.pointer("/Throttle/ActiveControlType").is_none());
    }

    #[test]
    fn gan_mppt_efficiency_skipped_when_input_power_below_threshold() {
        let mut root = json!({
            "GanMppt": {
                "Id0": {
                    "Power": {
                        "input_voltage": 0.1, "input_current": 0.1,
                        "output_voltage": 0.05, "output_current": 0.05
                    }
                }
            }
        });
        apply_derived(&mut root);
        assert!(
            root.pointer("/GanMppt/Id0/Power/input_power")
                .and_then(Value::as_f64)
                .is_some()
        );
        assert!(root.pointer("/GanMppt/Id0/Power/efficiency").is_none());
    }

    #[test]
    fn gan_mppt_efficiency_present_above_threshold() {
        let mut root = json!({
            "GanMppt": {
                "Id1": {
                    "Power": {
                        "input_voltage": 50.0, "input_current": 2.0,
                        "output_voltage": 48.0, "output_current": 2.0
                    }
                }
            }
        });
        apply_derived(&mut root);
        let eff = root
            .pointer("/GanMppt/Id1/Power/efficiency")
            .and_then(Value::as_f64)
            .expect("efficiency present");
        assert!((eff - 96.0).abs() < 1e-3, "got {eff}");
    }
}
