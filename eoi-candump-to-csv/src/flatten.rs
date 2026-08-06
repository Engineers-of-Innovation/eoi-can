use eoi_can_decoder::*;

use crate::filter::Filter;

pub fn column_schema(filter: &Filter) -> Vec<String> {
    let mut cols = Vec::new();

    if filter.battery {
        cols.extend(battery_columns().iter().map(|s| s.to_string()));
    }
    if filter.vesc {
        cols.extend(vesc_columns().iter().map(|s| s.to_string()));
    }
    if filter.throttle {
        cols.extend(throttle_columns().iter().map(|s| s.to_string()));
    }
    for n in filter.mppt_instances() {
        cols.extend(mppt_columns(n));
    }
    for n in filter.gan_mppt_instances() {
        cols.extend(gan_mppt_columns(n));
    }
    if filter.gnss {
        cols.extend(gnss_columns().iter().map(|s| s.to_string()));
    }
    if filter.rudder {
        cols.extend(rudder_columns().iter().map(|s| s.to_string()));
    }
    if filter.height {
        cols.extend(height_columns().iter().map(|s| s.to_string()));
    }
    if filter.temperature {
        cols.extend(temperature_columns().iter().map(|s| s.to_string()));
    }
    cols
}

pub fn flatten(data: &EoiCanData, out: &mut Vec<(String, String)>) {
    out.clear();
    match data {
        EoiCanData::EoiBattery(b) => flatten_battery(b, out),
        EoiCanData::Vesc(v) => flatten_vesc(v, out),
        EoiCanData::Throttle(t) => flatten_throttle(t, out),
        EoiCanData::Mppt(m) => flatten_mppt(m, out),
        EoiCanData::GanMppt(m) => flatten_gan_mppt(m, out),
        EoiCanData::Gnss(g) => flatten_gnss(g, out),
        EoiCanData::RudderController(r) => flatten_rudder(r, out),
        EoiCanData::HeightSensors(h) => flatten_height(h, out),
        EoiCanData::Temperature(t) => flatten_temperature(t, out),
    }
}

// ---------- Battery ----------

fn battery_columns() -> &'static [&'static str] {
    &[
        "battery.pack_current",
        "battery.perri_current",
        "battery.charge_current",
        "battery.discharge_current",
        "battery.soc",
        "battery.error_flags",
        "battery.balancing_status",
        "battery.cell_voltage_1",
        "battery.cell_voltage_2",
        "battery.cell_voltage_3",
        "battery.cell_voltage_4",
        "battery.cell_voltage_5",
        "battery.cell_voltage_6",
        "battery.cell_voltage_7",
        "battery.cell_voltage_8",
        "battery.cell_voltage_9",
        "battery.cell_voltage_10",
        "battery.cell_voltage_11",
        "battery.cell_voltage_12",
        "battery.cell_voltage_13",
        "battery.cell_voltage_14",
        "battery.pack_voltage",
        "battery.stack_voltage",
        "battery.temp_1",
        "battery.temp_2",
        "battery.temp_3",
        "battery.temp_4",
        "battery.ic_temperature",
        "battery.battery_state",
        "battery.charge_state",
        "battery.discharge_state",
        "battery.uptime_ms",
    ]
}

fn flatten_battery(b: &EoiBattery, out: &mut Vec<(String, String)>) {
    match b {
        EoiBattery::PackAndPerriCurrent(d) => {
            out.push(("battery.pack_current".into(), fmt_f(d.pack_current)));
            out.push(("battery.perri_current".into(), fmt_f(d.perri_current)));
        }
        EoiBattery::ChargeAndDischargeCurrent(d) => {
            out.push(("battery.charge_current".into(), fmt_f(d.charge_current)));
            out.push((
                "battery.discharge_current".into(),
                fmt_f(d.discharge_current),
            ));
        }
        EoiBattery::SocErrorFlagsAndBalancing(d) => {
            out.push(("battery.soc".into(), fmt_f(d.state_of_charge)));
            out.push(("battery.error_flags".into(), d.error_flags.to_string()));
            out.push((
                "battery.balancing_status".into(),
                d.balancing_status.to_string(),
            ));
        }
        EoiBattery::CellVoltages1_4(d) => push_cells(out, 1, &d.cell_voltage),
        EoiBattery::CellVoltages5_8(d) => push_cells(out, 5, &d.cell_voltage),
        EoiBattery::CellVoltages9_12(d) => push_cells(out, 9, &d.cell_voltage),
        EoiBattery::CellVoltages13_14PackAndStack(d) => {
            push_cells(out, 13, &d.cell_voltage);
            out.push(("battery.pack_voltage".into(), fmt_f(d.pack_voltage)));
            out.push(("battery.stack_voltage".into(), fmt_f(d.stack_voltage)));
        }
        EoiBattery::TemperaturesAndStates(d) => {
            for (i, t) in d.temperatures.iter().enumerate() {
                out.push((format!("battery.temp_{}", i + 1), t.to_string()));
            }
            out.push((
                "battery.ic_temperature".into(),
                d.ic_temperature.to_string(),
            ));
            out.push((
                "battery.battery_state".into(),
                format!("{:?}", d.battery_state),
            ));
            out.push((
                "battery.charge_state".into(),
                format!("{:?}", d.charge_state),
            ));
            out.push((
                "battery.discharge_state".into(),
                format!("{:?}", d.discharge_state),
            ));
        }
        EoiBattery::BatteryUptime(d) => {
            out.push(("battery.uptime_ms".into(), d.uptime_ms.to_string()));
        }
    }
}

fn push_cells(out: &mut Vec<(String, String)>, start: usize, cells: &[f32]) {
    for (i, v) in cells.iter().enumerate() {
        out.push((format!("battery.cell_voltage_{}", start + i), fmt_f(*v)));
    }
}

// ---------- VESC ----------

fn vesc_columns() -> &'static [&'static str] {
    &[
        "vesc.rpm",
        "vesc.total_current",
        "vesc.duty_cycle",
        "vesc.amp_hours_used",
        "vesc.amp_hours_generated",
        "vesc.watt_hours_used",
        "vesc.watt_hours_generated",
        "vesc.fet_temp",
        "vesc.motor_temp",
        "vesc.total_input_current",
        "vesc.current_pid_position",
        "vesc.input_voltage",
        "vesc.tachometer",
    ]
}

fn flatten_vesc(v: &VescData, out: &mut Vec<(String, String)>) {
    match v {
        VescData::StatusMessage1 {
            rpm,
            total_current,
            duty_cycle,
        } => {
            out.push(("vesc.rpm".into(), rpm.to_string()));
            out.push(("vesc.total_current".into(), fmt_f(*total_current)));
            out.push(("vesc.duty_cycle".into(), fmt_f(*duty_cycle)));
        }
        VescData::StatusMessage2 {
            amp_hours_used,
            amp_hours_generated,
        } => {
            out.push(("vesc.amp_hours_used".into(), fmt_f(*amp_hours_used)));
            out.push((
                "vesc.amp_hours_generated".into(),
                fmt_f(*amp_hours_generated),
            ));
        }
        VescData::StatusMessage3 {
            watt_hours_used,
            watt_hours_generated,
        } => {
            out.push(("vesc.watt_hours_used".into(), fmt_f(*watt_hours_used)));
            out.push((
                "vesc.watt_hours_generated".into(),
                fmt_f(*watt_hours_generated),
            ));
        }
        VescData::StatusMessage4 {
            fet_temp,
            motor_temp,
            total_input_current,
            current_pid_position,
        } => {
            out.push(("vesc.fet_temp".into(), fmt_f(*fet_temp)));
            out.push(("vesc.motor_temp".into(), fmt_f(*motor_temp)));
            out.push((
                "vesc.total_input_current".into(),
                fmt_f(*total_input_current),
            ));
            out.push((
                "vesc.current_pid_position".into(),
                fmt_f(*current_pid_position),
            ));
        }
        VescData::StatusMessage5 {
            input_voltage,
            tachometer,
        } => {
            out.push(("vesc.input_voltage".into(), fmt_f(*input_voltage)));
            out.push(("vesc.tachometer".into(), tachometer.to_string()));
        }
    }
}

// ---------- Throttle ----------

fn throttle_columns() -> &'static [&'static str] {
    &[
        "throttle.duty_cycle",
        "throttle.current",
        "throttle.current_relative",
        "throttle.rpm",
        "throttle.status.value",
        "throttle.status.raw_angle",
        "throttle.status.raw_deadmen",
        "throttle.status.gain",
        "throttle.status.error",
        "throttle.config.control_type",
        "throttle.config.lever_forward",
        "throttle.config.lever_backward",
    ]
}

fn flatten_throttle(t: &ThrottleData, out: &mut Vec<(String, String)>) {
    match t {
        ThrottleData::ToVescDutyCycle(v) => out.push(("throttle.duty_cycle".into(), fmt_f(*v))),
        ThrottleData::ToVescCurrent(v) => out.push(("throttle.current".into(), fmt_f(*v))),
        ThrottleData::ToVescCurrentRelative(v) => {
            out.push(("throttle.current_relative".into(), fmt_f(*v)))
        }
        ThrottleData::ToVescRpm(v) => out.push(("throttle.rpm".into(), fmt_f(*v))),
        ThrottleData::Status(s) => {
            out.push(("throttle.status.value".into(), fmt_f(s.value)));
            out.push(("throttle.status.raw_angle".into(), s.raw_angle.to_string()));
            out.push((
                "throttle.status.raw_deadmen".into(),
                s.raw_deadmen.to_string(),
            ));
            out.push(("throttle.status.gain".into(), s.gain.to_string()));
            out.push(("throttle.status.error".into(), s.error.to_string()));
        }
        ThrottleData::Config(c) => {
            out.push((
                "throttle.config.control_type".into(),
                format!("{:?}", c.control_type),
            ));
            out.push((
                "throttle.config.lever_forward".into(),
                c.lever_forward.to_string(),
            ));
            out.push((
                "throttle.config.lever_backward".into(),
                c.lever_backward.to_string(),
            ));
        }
    }
}

// ---------- MPPT ----------

fn mppt_columns(n: u8) -> Vec<String> {
    let mut cols = Vec::with_capacity(28);
    for ch in 0..4 {
        cols.push(format!("mppt{n}.ch{ch}.voltage_in"));
        cols.push(format!("mppt{n}.ch{ch}.current_in"));
        cols.push(format!("mppt{n}.ch{ch}.duty_cycle"));
        cols.push(format!("mppt{n}.ch{ch}.algorithm"));
        cols.push(format!("mppt{n}.ch{ch}.algorithm_state"));
        cols.push(format!("mppt{n}.ch{ch}.channel_active"));
    }
    cols.push(format!("mppt{n}.voltage_out"));
    cols.push(format!("mppt{n}.current_out"));
    cols.push(format!("mppt{n}.voltage_out_switch"));
    cols.push(format!("mppt{n}.temperature"));
    cols.push(format!("mppt{n}.state"));
    cols.push(format!("mppt{n}.pwm_enabled"));
    cols.push(format!("mppt{n}.switch_on"));
    cols
}

fn flatten_mppt(m: &MpptData, out: &mut Vec<(String, String)>) {
    let n = m.node_id();
    match m.inner() {
        MpptInfo::Channel0(c) => push_mppt_channel(out, n, 0, c),
        MpptInfo::Channel1(c) => push_mppt_channel(out, n, 1, c),
        MpptInfo::Channel2(c) => push_mppt_channel(out, n, 2, c),
        MpptInfo::Channel3(c) => push_mppt_channel(out, n, 3, c),
        MpptInfo::ChannelUnknown(_) => {}
        MpptInfo::Power(p) => {
            out.push((format!("mppt{n}.voltage_out"), fmt_f(p.voltage_out)));
            out.push((format!("mppt{n}.current_out"), fmt_f(p.current_out)));
        }
        MpptInfo::Status(s) => {
            out.push((
                format!("mppt{n}.voltage_out_switch"),
                fmt_f(s.voltage_out_switch),
            ));
            out.push((format!("mppt{n}.temperature"), s.temperature.to_string()));
            out.push((format!("mppt{n}.state"), s.state.to_string()));
            out.push((format!("mppt{n}.pwm_enabled"), s.pwm_enabled.to_string()));
            out.push((format!("mppt{n}.switch_on"), s.switch_on.to_string()));
        }
    }
}

fn push_mppt_channel(out: &mut Vec<(String, String)>, n: u8, ch: u8, c: &MpptChannel) {
    match c {
        MpptChannel::Power(p) => {
            out.push((format!("mppt{n}.ch{ch}.voltage_in"), fmt_f(p.voltage_in)));
            out.push((format!("mppt{n}.ch{ch}.current_in"), fmt_f(p.current_in)));
        }
        MpptChannel::State(s) => {
            out.push((
                format!("mppt{n}.ch{ch}.duty_cycle"),
                s.duty_cycle.to_string(),
            ));
            out.push((format!("mppt{n}.ch{ch}.algorithm"), s.algorithm.to_string()));
            out.push((
                format!("mppt{n}.ch{ch}.algorithm_state"),
                s.algorithm_state.to_string(),
            ));
            out.push((
                format!("mppt{n}.ch{ch}.channel_active"),
                s.channel_active.to_string(),
            ));
        }
    }
}

// ---------- GaN MPPT ----------

fn gan_mppt_columns(n: u8) -> Vec<String> {
    vec![
        format!("gan_mppt{n}.input_voltage"),
        format!("gan_mppt{n}.input_current"),
        format!("gan_mppt{n}.output_voltage"),
        format!("gan_mppt{n}.output_current"),
        format!("gan_mppt{n}.mode"),
        format!("gan_mppt{n}.fault"),
        format!("gan_mppt{n}.enabled"),
        format!("gan_mppt{n}.board_temp"),
        format!("gan_mppt{n}.heat_sink_temp"),
        format!("gan_mppt{n}.sweep.index"),
        format!("gan_mppt{n}.sweep.current"),
        format!("gan_mppt{n}.sweep.voltage"),
    ]
}

fn flatten_gan_mppt(m: &GanMpptData, out: &mut Vec<(String, String)>) {
    let n = m.node_id();
    match m.inner() {
        GanMpptPacket::Power(p) => {
            out.push((format!("gan_mppt{n}.input_voltage"), fmt_f(p.input_voltage)));
            out.push((format!("gan_mppt{n}.input_current"), fmt_f(p.input_current)));
            out.push((
                format!("gan_mppt{n}.output_voltage"),
                fmt_f(p.output_voltage),
            ));
            out.push((
                format!("gan_mppt{n}.output_current"),
                fmt_f(p.output_current),
            ));
        }
        GanMpptPacket::Status(s) => {
            out.push((format!("gan_mppt{n}.mode"), format!("{:?}", s.mode)));
            out.push((format!("gan_mppt{n}.fault"), format!("{:?}", s.fault)));
            out.push((format!("gan_mppt{n}.enabled"), s.enabled.to_string()));
            out.push((format!("gan_mppt{n}.board_temp"), s.board_temp.to_string()));
            out.push((
                format!("gan_mppt{n}.heat_sink_temp"),
                s.heat_sink_temp.to_string(),
            ));
        }
        GanMpptPacket::SweepData(s) => {
            out.push((format!("gan_mppt{n}.sweep.index"), s.index.to_string()));
            out.push((format!("gan_mppt{n}.sweep.current"), fmt_f(s.current)));
            out.push((format!("gan_mppt{n}.sweep.voltage"), fmt_f(s.voltage)));
        }
    }
}

// ---------- GNSS ----------

fn gnss_columns() -> &'static [&'static str] {
    &[
        "gnss.fix",
        "gnss.sats",
        "gnss.sats_used",
        "gnss.speed",
        "gnss.heading",
        "gnss.latitude",
        "gnss.longitude",
        "gnss.datetime",
    ]
}

fn flatten_gnss(g: &GnssData, out: &mut Vec<(String, String)>) {
    match g {
        GnssData::GnssStatus(s) => {
            out.push(("gnss.fix".into(), s.fix.to_string()));
            out.push(("gnss.sats".into(), s.sats.to_string()));
            out.push(("gnss.sats_used".into(), s.sats_used.to_string()));
        }
        GnssData::GnssSpeedAndHeading(speed, heading) => {
            out.push(("gnss.speed".into(), fmt_f(*speed)));
            out.push(("gnss.heading".into(), fmt_f(*heading)));
        }
        GnssData::GnssLatitude(v) => out.push(("gnss.latitude".into(), fmt_f64(*v))),
        GnssData::GnssLongitude(v) => out.push(("gnss.longitude".into(), fmt_f64(*v))),
        GnssData::GnssDateTime(d) => {
            out.push((
                "gnss.datetime".into(),
                format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                    d.year, d.month, d.day, d.hours, d.minutes, d.seconds
                ),
            ));
        }
    }
}

// ---------- Rudder ----------

fn rudder_columns() -> &'static [&'static str] {
    &[
        "rudder.servo.setpoint",
        "rudder.servo.state",
        "rudder.servo.command",
        "rudder.cooling_pump",
        "rudder.steering.angle",
        "rudder.steering.raw_adc",
        "rudder.flow_in.flow_rate",
        "rudder.flow_in.temperature",
        "rudder.flow_in.raw_pulses",
        "rudder.flow_in.raw_adc",
        "rudder.flow_out.flow_rate",
        "rudder.flow_out.temperature",
        "rudder.flow_out.raw_pulses",
        "rudder.flow_out.raw_adc",
        "rudder.motor_temp.temperature",
        "rudder.motor_temp.raw_adc",
    ]
}

fn flatten_rudder(r: &RudderControllerData, out: &mut Vec<(String, String)>) {
    match r {
        RudderControllerData::Servo(s) => match s {
            ServoData::Setpoint(v) => out.push(("rudder.servo.setpoint".into(), v.to_string())),
            ServoData::Status(st) => {
                out.push(("rudder.servo.state".into(), format!("{:?}", st.state)));
                out.push(("rudder.servo.setpoint".into(), st.setpoint.to_string()));
            }
            ServoData::Command(c) => out.push(("rudder.servo.command".into(), format!("{c:?}"))),
        },
        RudderControllerData::CoolingPumpStatus(s) => {
            out.push(("rudder.cooling_pump".into(), format!("{s:?}")));
        }
        RudderControllerData::SteeringAngle(s) => {
            out.push(("rudder.steering.angle".into(), s.angle.to_string()));
            out.push(("rudder.steering.raw_adc".into(), s.raw_adc.to_string()));
        }
        RudderControllerData::FlowSensorIn(f) => push_flow_sensor(out, "flow_in", f),
        RudderControllerData::FlowSensorOut(f) => push_flow_sensor(out, "flow_out", f),
        RudderControllerData::MotorTemperature(t) => {
            if let Some(v) = t.temperature {
                out.push(("rudder.motor_temp.temperature".into(), fmt_f(v)));
            }
            out.push(("rudder.motor_temp.raw_adc".into(), t.raw_adc.to_string()));
        }
    }
}

fn push_flow_sensor(out: &mut Vec<(String, String)>, key: &str, f: &FlowSensor) {
    out.push((format!("rudder.{key}.flow_rate"), f.flow_rate.to_string()));
    if let Some(v) = f.temperature {
        out.push((format!("rudder.{key}.temperature"), fmt_f(v)));
    }
    out.push((format!("rudder.{key}.raw_pulses"), f.raw_pulses.to_string()));
    out.push((format!("rudder.{key}.raw_adc"), f.raw_adc.to_string()));
}

// ---------- Height sensors ----------

fn height_columns() -> &'static [&'static str] {
    &[
        "height.front_left.state",
        "height.front_left.value",
        "height.front_right.state",
        "height.front_right.value",
        "height.reserved1.state",
        "height.reserved1.value",
        "height.reserved2.state",
        "height.reserved2.value",
    ]
}

fn flatten_height(h: &HeightSensorData, out: &mut Vec<(String, String)>) {
    let (prefix, status) = match h {
        HeightSensorData::FrontLeft(s) => ("front_left", s),
        HeightSensorData::FrontRight(s) => ("front_right", s),
        HeightSensorData::Reserved1(s) => ("reserved1", s),
        HeightSensorData::Reserved2(s) => ("reserved2", s),
    };
    out.push((
        format!("height.{prefix}.state"),
        format!("{:?}", status.state),
    ));
    out.push((format!("height.{prefix}.value"), status.value.to_string()));
}

// ---------- Temperature ----------

fn temperature_columns() -> &'static [&'static str] {
    &[
        "temperature.height_sensors_controller",
        "temperature.rudder_controller",
    ]
}

fn flatten_temperature(t: &TemperatureData, out: &mut Vec<(String, String)>) {
    match t {
        TemperatureData::HeightSensorsController(v) => {
            out.push(("temperature.height_sensors_controller".into(), fmt_f(*v)));
        }
        TemperatureData::RudderController(v) => {
            out.push(("temperature.rudder_controller".into(), fmt_f(*v)));
        }
        TemperatureData::MotorNtc(ntc) => {
            // A faulted frame emits no temperature column at all, so a gap in the CSV
            // is a fault rather than a frame that never arrived -- the status columns
            // beside it are still there to say which fault.
            if let Some(v) = ntc.temperature {
                out.push(("temperature.motor_ntc".into(), fmt_f(v)));
            }
            let s = &ntc.status;
            for (name, flag) in [
                ("sensor_open", s.sensor_open),
                ("sensor_short", s.sensor_short),
                ("out_of_range", s.out_of_range),
                ("settling", s.settling),
                ("acquisition_error", s.acquisition_error),
                ("previous_tx_failed", s.previous_tx_failed),
            ] {
                out.push((format!("temperature.motor_ntc.{name}"), flag.to_string()));
            }
            if let Some(counter) = ntc.frame_counter {
                out.push((
                    "temperature.motor_ntc.frame_counter".into(),
                    counter.to_string(),
                ));
            }
        }
    }
}

// ---------- formatting helpers ----------

fn fmt_f(v: f32) -> String {
    if v.is_nan() || v.is_infinite() {
        return v.to_string();
    }
    format!("{v}")
}

fn fmt_f64(v: f64) -> String {
    if v.is_nan() || v.is_infinite() {
        return v.to_string();
    }
    format!("{v}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_can::{Id, StandardId};
    use eoi_can_decoder::can_frame::CanFrame;

    fn decode(id: u16, data: &[u8]) -> EoiCanData {
        let frame = CanFrame::from_encoded(Id::Standard(StandardId::new(id).unwrap()), data);
        parse_eoi_can_data(&frame).expect("decode failed")
    }

    fn flat(id: u16, data: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        flatten(&decode(id, data), &mut out);
        out
    }

    #[test]
    fn pack_and_perri_current_columns() {
        // Reuses the constants from eoi-can-decoder's own tests
        let pairs = flat(0x100, &0x5817DA41EBF577BE_u64.to_be_bytes());
        let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["battery.pack_current", "battery.perri_current"]);
    }

    #[test]
    fn cell_voltages_1_4_named_correctly() {
        let pairs = flat(0x103, &0x36102C102D103710_u64.to_be_bytes());
        let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "battery.cell_voltage_1",
                "battery.cell_voltage_2",
                "battery.cell_voltage_3",
                "battery.cell_voltage_4",
            ]
        );
        assert_eq!(pairs[0].1, "4.15");
    }

    #[test]
    fn mppt_status_uses_node_id() {
        // mppt id 1, info field 9 -> CAN id 0x719
        let data = [0x00, 0x00, 0x80, 0x42, 0x10, 0x00, 0x05, 0x03];
        let pairs = flat(0x719, &data);
        for (k, _) in &pairs {
            assert!(k.starts_with("mppt1."), "unexpected key: {k}");
        }
    }

    #[test]
    fn schema_order_stable() {
        let f = Filter {
            battery: true,
            gnss: true,
            mppt: Some(vec![1u8, 2].into_iter().collect()),
            ..Default::default()
        };
        let schema = column_schema(&f);
        // battery columns come before mppt before gnss
        let pos = |s: &str| schema.iter().position(|c| c == s).unwrap();
        assert!(pos("battery.pack_current") < pos("mppt1.voltage_out"));
        assert!(pos("mppt1.voltage_out") < pos("mppt2.voltage_out"));
        assert!(pos("mppt2.voltage_out") < pos("gnss.latitude"));
    }
}
