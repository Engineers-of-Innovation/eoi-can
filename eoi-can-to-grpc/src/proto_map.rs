use crate::live_state::{Quality, SnapshotView};
use crate::pb::eoi::telemetry::v1;

pub fn to_proto(view: &SnapshotView, seq: u64, session_id: &str) -> v1::Snapshot {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    v1::Snapshot {
        seq,
        t: Some(prost_types::Timestamp {
            seconds: now.as_secs() as i64,
            nanos: now.subsec_nanos() as i32,
        }),
        boat_uptime_ms: view.boat_uptime_ms.unwrap_or(0),
        session_id: session_id.to_string(),
        power: Some(v1::Power {
            net_w: view.net_w,
            in_w: view.in_w,
            out_w: view.out_w,
            pack_v: view.pack_v,
        }),
        battery: Some(v1::Battery {
            soc_pct: view.soc_pct,
            pack_v: view.pack_v,
            stack_v: view.stack_v,
            pack_a: view.pack_a,
            peri_a: view.peri_a,
            charge_a: view.charge_a,
            discharge_a: view.discharge_a,
            cell_v: view.cell_v.clone(),
            temps: view
                .batt_temps
                .iter()
                .map(|(label, c, q)| v1::Temperature {
                    label: label.clone(),
                    celsius: *c,
                    quality: quality(*q).into(),
                })
                .collect(),
            state: view.batt_state,
            charge_state: view.charge_state,
            discharge_state: view.discharge_state,
            error_flags: view.error_flags,
            balancing_flags: view.balancing_flags,
        }),
        gnss: Some(v1::Gnss {
            lat: view.lat,
            lon: view.lon,
            speed_kmh: view.speed_kmh,
            heading_deg: view.heading_deg,
            fix: view.gnss_fix,
            sats: view.gnss_sats,
            sats_used: view.gnss_sats_used,
        }),
        motor: Some(v1::Motor {
            rpm: view.rpm,
            current_a: view.motor_current_a,
            duty_pct: view.motor_duty,
            tacho: view.tacho,
            fet_celsius: view.fet_celsius,
            motor_celsius: view.motor_celsius,
            motor_temp_quality: quality(view.motor_temp_quality).into(),
            ah_used: view.ah_used,
            ah_gen: view.ah_gen,
            wh_used: view.wh_used,
            wh_gen: view.wh_gen,
            input_v: view.input_v,
        }),
        rudder: Some(v1::Rudder {
            setpoint: view.rudder_setpoint,
            actual: view.rudder_actual,
            steering_deg: view.steering_deg,
            state: view.rudder_state,
            fault: view.rudder_fault,
            controller_celsius: view.rudder_ctl_c,
            height_controller_celsius: view.height_ctl_c,
            flow_in_ml_min: view.flow_in,
            flow_out_ml_min: view.flow_out,
        }),
        throttle: Some(v1::Throttle {
            duty_pct: view.throttle_duty,
            current_a: view.throttle_current,
            rpm: view.throttle_rpm,
            current_rel_pct: view.throttle_rel,
            position_pct: view.throttle_pos,
            error_flags: view.throttle_errors,
        }),
        mppt: view
            .mppts
            .iter()
            .map(|m| v1::Mppt {
                label: m.label.clone(),
                id: m.id,
                gan: m.gan,
                vin: m.vin,
                iin: m.iin,
                vout: m.vout,
                iout: m.iout,
                celsius: m.celsius,
                state: m.state,
                flags: m.flags,
            })
            .collect(),
        height: view
            .heights
            .iter()
            .map(|h| v1::HeightSensor {
                label: h.label.clone(),
                state: h.state,
                height: h.height,
            })
            .collect(),
        hottest_mppt: view
            .hottest_mppt
            .as_ref()
            .map(|(label, c)| v1::Temperature {
                label: label.clone(),
                celsius: Some(*c),
                quality: v1::Quality::Ok.into(),
            }),
        hottest_battery: view
            .hottest_battery
            .as_ref()
            .map(|(label, c)| v1::Temperature {
                label: label.clone(),
                celsius: Some(*c),
                quality: v1::Quality::Ok.into(),
            }),
        warnings: Some(v1::Warnings {
            battery_state: view.warnings.battery_state,
            charge_fet: view.warnings.charge_fet,
            discharge_fet: view.warnings.discharge_fet,
            soc_low: view.warnings.soc_low,
            motor_hot: view.warnings.motor_hot,
            fet_hot: view.warnings.fet_hot,
            mppt_hot: view.warnings.mppt_hot,
            battery_hot: view.warnings.battery_hot,
            throttle: view.warnings.throttle,
        }),
        bus: Some(v1::BusHealth {
            frames_per_sec: view.bus.frames_per_sec,
            frames_total: view.bus.frames_total,
            frames_unknown: view.bus.frames_unknown,
            decode_errors: view.bus.decode_errors,
        }),
    }
}

fn quality(q: Quality) -> v1::Quality {
    match q {
        Quality::Ok => v1::Quality::Ok,
        Quality::Clamped => v1::Quality::Clamped,
        Quality::Settling => v1::Quality::Settling,
        Quality::Absent => v1::Quality::Absent,
    }
}
