//! Home Assistant MQTT Discovery registry and publisher.
//!
//! Adding a new entity: append one `sensor(...)` call to the appropriate
//! `add_*` function below. Value templates must address a real leaf in the
//! merged JSON published to the state topic — the `registry_matches_json_leaves`
//! test will fail if they drift.
//!
//! `device_class` values must match Home Assistant's built-in list:
//! <https://www.home-assistant.io/integrations/sensor/#device-class>
//! `state_class` is one of `measurement`, `total`, `total_increasing`.

use std::borrow::Cow;

use mppt_layout::{LAYOUT, MpptKind};
use paho_mqtt as mqtt;
use serde_json::{Map, Value, json};

use crate::mqtt_settings;

type Str = Cow<'static, str>;

#[derive(Clone, Copy)]
struct Device {
    id: &'static str,
    name: &'static str,
}

const HUB: Device = Device {
    id: mqtt_settings::DEVICE_ID,
    name: mqtt_settings::DEVICE_NAME,
};
const BATTERY: Device = Device {
    id: "eoi_battery",
    name: "Battery",
};
const VESC: Device = Device {
    id: "eoi_vesc",
    name: "VESC",
};
const THROTTLE: Device = Device {
    id: "eoi_throttle",
    name: "Throttle",
};
const RUDDER: Device = Device {
    id: "eoi_rudder",
    name: "Rudder",
};
const MOTOR: Device = Device {
    id: "eoi_motor",
    name: "Motor",
};
const HEIGHT_SENSORS: Device = Device {
    id: "eoi_height_sensors",
    name: "Height Sensors",
};
const GNSS: Device = Device {
    id: "eoi_gnss",
    name: "GNSS",
};
const MPPT: Device = Device {
    id: "eoi_mppt",
    name: "MPPT",
};
const GAN_MPPT: Device = Device {
    id: "eoi_gan_mppt",
    name: "GaN MPPT",
};
const SOLAR_PANELS: Device = Device {
    id: "eoi_solar_panels",
    name: "Solar Panels",
};

pub struct HaEntity {
    component: Str,
    object_id: Str,
    name: Str,
    path: Str,
    value_template: Str,
    unit: Option<Str>,
    device_class: Option<Str>,
    state_class: Option<Str>,
    icon: Option<Str>,
    entity_category: Option<Str>,
    enabled_by_default: Option<bool>,
    json_attributes_template: Option<Str>,
    suggested_display_precision: Option<u8>,
    device: Device,
}

fn sensor(object_id: impl Into<Str>, name: impl Into<Str>, path: &str) -> HaEntity {
    let chain = path_defined_chain(path);
    HaEntity {
        component: Cow::Borrowed("sensor"),
        object_id: object_id.into(),
        name: name.into(),
        path: Cow::Owned(path.to_string()),
        value_template: Cow::Owned(format!(
            "{{% if {chain} %}}{{{{ value_json.{path} }}}}{{% endif %}}"
        )),
        unit: None,
        device_class: None,
        state_class: None,
        icon: None,
        entity_category: None,
        enabled_by_default: None,
        json_attributes_template: None,
        suggested_display_precision: None,
        device: HUB,
    }
}

fn binary_sensor(object_id: impl Into<Str>, name: impl Into<Str>, path: &str) -> HaEntity {
    let chain = path_defined_chain(path);
    HaEntity {
        component: Cow::Borrowed("binary_sensor"),
        object_id: object_id.into(),
        name: name.into(),
        path: Cow::Owned(path.to_string()),
        value_template: Cow::Owned(format!(
            "{{% if {chain} %}}{{{{ 'ON' if value_json.{path} else 'OFF' }}}}{{% endif %}}"
        )),
        unit: None,
        device_class: None,
        state_class: None,
        icon: None,
        entity_category: None,
        enabled_by_default: None,
        json_attributes_template: None,
        suggested_display_precision: None,
        device: HUB,
    }
}

fn device_tracker(object_id: impl Into<Str>, name: impl Into<Str>, path: &str) -> HaEntity {
    let chain = path_defined_chain(path);
    HaEntity {
        component: Cow::Borrowed("device_tracker"),
        object_id: object_id.into(),
        name: name.into(),
        path: Cow::Owned(path.to_string()),
        value_template: Cow::Owned(format!(
            "{{% if {chain} and value_json.{path} != 0 %}}home{{% else %}}not_home{{% endif %}}"
        )),
        unit: None,
        device_class: None,
        state_class: None,
        icon: None,
        entity_category: None,
        enabled_by_default: None,
        json_attributes_template: None,
        suggested_display_precision: None,
        device: HUB,
    }
}

fn path_defined_chain(path: &str) -> String {
    let mut accum = String::new();
    let mut checks: Vec<String> = Vec::new();
    for seg in path.split('.') {
        if !accum.is_empty() {
            accum.push('.');
        }
        accum.push_str(seg);
        checks.push(format!("value_json.{accum} is defined"));
    }
    checks.join(" and ")
}

fn availability_template_for(path: &str) -> String {
    let chain = path_defined_chain(path);
    format!("{{% if {chain} %}}online{{% else %}}offline{{% endif %}}")
}

struct Scope<'a> {
    out: &'a mut Vec<HaEntity>,
    device: Device,
}

impl Scope<'_> {
    fn push(&mut self, mut e: HaEntity) {
        e.device = self.device;
        self.out.push(e);
    }
}

fn scoped(out: &mut Vec<HaEntity>, device: Device) -> Scope<'_> {
    Scope { out, device }
}

impl HaEntity {
    fn unit(mut self, v: impl Into<Str>) -> Self {
        self.unit = Some(v.into());
        self
    }
    fn device_class(mut self, v: impl Into<Str>) -> Self {
        self.device_class = Some(v.into());
        self
    }
    fn state_class(mut self, v: impl Into<Str>) -> Self {
        self.state_class = Some(v.into());
        self
    }
    fn icon(mut self, v: impl Into<Str>) -> Self {
        self.icon = Some(v.into());
        self
    }
    fn diagnostic(mut self) -> Self {
        self.entity_category = Some(Cow::Borrowed("diagnostic"));
        self
    }
    fn disabled(mut self) -> Self {
        self.enabled_by_default = Some(false);
        self
    }
    fn json_attributes_template(mut self, v: impl Into<Str>) -> Self {
        self.json_attributes_template = Some(v.into());
        self
    }
    fn display_precision(mut self, n: u8) -> Self {
        self.suggested_display_precision = Some(n);
        self
    }
    fn measurement(self) -> Self {
        self.state_class("measurement")
    }
    fn total_increasing(self) -> Self {
        self.state_class("total_increasing")
    }

    fn voltage(self) -> Self {
        self.unit("V").device_class("voltage").measurement()
    }
    fn current(self) -> Self {
        self.unit("A").device_class("current").measurement()
    }
    fn power(self) -> Self {
        self.unit("W").device_class("power").measurement()
    }
    fn efficiency(self) -> Self {
        self.unit("%")
            .measurement()
            .icon("mdi:chart-bell-curve-cumulative")
    }
    fn temperature(self) -> Self {
        self.unit("°C").device_class("temperature").measurement()
    }
    fn energy_wh(self) -> Self {
        self.unit("Wh").device_class("energy").total_increasing()
    }
    fn charge_ah(self) -> Self {
        // HA has no device_class for charge; use a fitting icon.
        self.unit("Ah")
            .total_increasing()
            .icon("mdi:battery-charging")
    }
}

pub fn availability_will() -> mqtt::Message {
    mqtt::Message::new_retained(mqtt_settings::AVAILABILITY_TOPIC, "offline", mqtt::QOS_1)
}

pub fn publish_availability(client: &mqtt::Client, online: bool) -> Result<(), mqtt::Error> {
    let payload = if online { "online" } else { "offline" };
    client.publish(mqtt::Message::new_retained(
        mqtt_settings::AVAILABILITY_TOPIC,
        payload,
        mqtt::QOS_1,
    ))
}

/// Entities that once existed and have to be deleted from Home Assistant. Discovery
/// configs are published retained, so dropping a row from `entities()` is not enough:
/// the broker keeps serving the old config and HA keeps the entity forever. Publishing
/// an empty retained payload to the same topic is what removes it.
///
/// Rows stay here indefinitely -- they cost one empty publish per start -- since we
/// cannot know whether every broker that ever saw the entity has been swept.
const RETIRED: &[(&str, Device, &str)] = &[
    // 0x217, the rudder controller's motor NTC. A stopgap that shared the flow-out
    // sensor's pin; motor temperature comes from the standalone node (0x219) now.
    ("sensor", RUDDER, "rudder_motor_temperature"),
    ("sensor", RUDDER, "rudder_motor_temperature_raw_adc"),
];

fn discovery_topic(component: &str, device_id: &str, object_id: &str) -> String {
    format!(
        "{}/{component}/{device_id}/{object_id}/config",
        mqtt_settings::DISCOVERY_PREFIX,
    )
}

pub fn publish_discovery(client: &mqtt::Client) -> Result<(), mqtt::Error> {
    for (component, device, object_id) in RETIRED {
        let topic = discovery_topic(component, device.id, object_id);
        client.publish(mqtt::Message::new_retained(topic, "", mqtt::QOS_1))?;
    }
    for entity in entities() {
        let topic = discovery_topic(&entity.component, entity.device.id, &entity.object_id);
        let payload = build_config(&entity).to_string();
        client.publish(mqtt::Message::new_retained(topic, payload, mqtt::QOS_1))?;
    }
    Ok(())
}

fn build_config(entity: &HaEntity) -> Value {
    let unique_id = format!("{}_{}", entity.device.id, entity.object_id);
    let mut m = Map::new();
    m.insert("name".into(), json!(entity.name.as_ref()));
    m.insert("unique_id".into(), json!(unique_id));
    m.insert("object_id".into(), json!(unique_id));
    m.insert("state_topic".into(), json!(mqtt_settings::TOPIC));
    m.insert(
        "value_template".into(),
        json!(entity.value_template.as_ref()),
    );
    if let Some(v) = &entity.unit {
        m.insert("unit_of_measurement".into(), json!(v.as_ref()));
    }
    if let Some(v) = &entity.device_class {
        m.insert("device_class".into(), json!(v.as_ref()));
    }
    if let Some(v) = &entity.state_class {
        m.insert("state_class".into(), json!(v.as_ref()));
    }
    if let Some(v) = &entity.icon {
        m.insert("icon".into(), json!(v.as_ref()));
    }
    if let Some(v) = &entity.entity_category {
        m.insert("entity_category".into(), json!(v.as_ref()));
    }
    if let Some(v) = entity.enabled_by_default {
        m.insert("enabled_by_default".into(), json!(v));
    }
    if let Some(v) = &entity.json_attributes_template {
        m.insert("json_attributes_topic".into(), json!(mqtt_settings::TOPIC));
        m.insert("json_attributes_template".into(), json!(v.as_ref()));
    }
    if let Some(v) = entity.suggested_display_precision {
        m.insert("suggested_display_precision".into(), json!(v));
    }
    m.insert("expire_after".into(), json!(10));
    m.insert(
        "availability".into(),
        json!([
            {
                "topic": mqtt_settings::AVAILABILITY_TOPIC,
                "payload_available": "online",
                "payload_not_available": "offline",
            },
            {
                "topic": mqtt_settings::TOPIC,
                "value_template": availability_template_for(&entity.path),
                "payload_available": "online",
                "payload_not_available": "offline",
            }
        ]),
    );
    m.insert("availability_mode".into(), json!("all"));

    let mut device_block = json!({
        "identifiers": [entity.device.id],
        "name": entity.device.name,
        "manufacturer": mqtt_settings::DEVICE_MANUFACTURER,
        "model": mqtt_settings::DEVICE_MODEL,
    });
    if entity.device.id != HUB.id {
        device_block
            .as_object_mut()
            .unwrap()
            .insert("via_device".into(), json!(HUB.id));
    }
    m.insert("device".into(), device_block);
    Value::Object(m)
}

pub fn entities() -> Vec<HaEntity> {
    let mut v = Vec::new();
    add_datalogger(&mut v);
    add_battery(&mut v);
    add_vesc(&mut v);
    add_throttle(&mut v);
    add_rudder(&mut v);
    add_motor(&mut v);
    add_height_sensors(&mut v);
    add_gnss(&mut v);
    add_boat_location(&mut v);
    add_mppt(&mut v);
    add_gan_mppt(&mut v);
    add_solar_panels(&mut v);
    v
}

fn add_solar_panels(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, SOLAR_PANELS);
    for (i, kind) in LAYOUT.iter().enumerate() {
        let pos = i + 1;
        let (voltage_path, current_path, power_path, temperature_path) = match kind {
            MpptKind::Legacy { node, channel } => (
                format!("Mppt.Id{node}.Channel{channel}.Power.voltage_in"),
                format!("Mppt.Id{node}.Channel{channel}.Power.current_in"),
                format!("Mppt.Id{node}.Channel{channel}.Power.power_in"),
                format!("Mppt.Id{node}.Status.temperature"),
            ),
            MpptKind::Gan { node } => (
                format!("GanMppt.Id{node}.Power.input_voltage"),
                format!("GanMppt.Id{node}.Power.input_current"),
                format!("GanMppt.Id{node}.Power.input_power"),
                format!("GanMppt.Id{node}.Status.heat_sink_temp"),
            ),
        };
        v.push(
            sensor(
                format!("panel_{pos}_voltage"),
                format!("Panel {pos} Voltage"),
                &voltage_path,
            )
            .voltage(),
        );
        v.push(
            sensor(
                format!("panel_{pos}_current"),
                format!("Panel {pos} Current"),
                &current_path,
            )
            .current(),
        );
        v.push(
            sensor(
                format!("panel_{pos}_power"),
                format!("Panel {pos} Power"),
                &power_path,
            )
            .power(),
        );
        v.push(
            sensor(
                format!("panel_{pos}_temperature"),
                format!("Panel {pos} Temperature"),
                &temperature_path,
            )
            .temperature(),
        );
    }
}

fn add_boat_location(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, HUB);
    v.push(device_tracker("boat_location", "Boat Location", "Gnss.GnssLatitude").json_attributes_template(
        "{{ {'latitude': value_json.Gnss.GnssLatitude, 'longitude': value_json.Gnss.GnssLongitude} | tojson }}",
    ));
}

fn add_datalogger(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, HUB);
    v.push(
        sensor(
            "datalogger_uptime_system",
            "System Uptime",
            "DataLogger.Uptime.System",
        )
        .unit("s")
        .device_class("duration")
        .diagnostic(),
    );
    v.push(
        sensor(
            "datalogger_uptime_process",
            "Bridge Uptime",
            "DataLogger.Uptime.Process",
        )
        .unit("s")
        .device_class("duration")
        .diagnostic(),
    );
    v.push(
        sensor(
            "datalogger_cpu_load_1m",
            "CPU Load (1m)",
            "DataLogger.CpuLoad1M",
        )
        .measurement()
        .icon("mdi:cpu-64-bit")
        .diagnostic(),
    );
    v.push(
        sensor(
            "datalogger_cpu_temp",
            "CPU Temperature",
            "DataLogger.CpuTemp",
        )
        .temperature()
        .diagnostic(),
    );
    v.push(
        sensor(
            "datalogger_memory_usage",
            "Memory Usage",
            "DataLogger.MemoryUsage",
        )
        .unit("%")
        .measurement()
        .icon("mdi:memory")
        .diagnostic(),
    );
    v.push(
        sensor("datalogger_wifi_ip", "WiFi IP", "DataLogger.WifiIp")
            .icon("mdi:ip-network")
            .diagnostic(),
    );
}

fn add_battery(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, BATTERY);
    // Currents
    v.push(
        sensor(
            "battery_pack_current",
            "Battery Pack Current",
            "EoiBattery.PackAndPerriCurrent.pack_current",
        )
        .current(),
    );
    v.push(
        sensor(
            "battery_perri_current",
            "Battery Peripheral Current",
            "EoiBattery.PackAndPerriCurrent.perri_current",
        )
        .current(),
    );
    v.push(
        sensor(
            "battery_charge_current",
            "Battery Charge Current",
            "EoiBattery.ChargeAndDischargeCurrent.charge_current",
        )
        .current(),
    );
    v.push(
        sensor(
            "battery_discharge_current",
            "Battery Discharge Current",
            "EoiBattery.ChargeAndDischargeCurrent.discharge_current",
        )
        .current(),
    );

    // SOC + flags
    v.push(
        sensor(
            "battery_soc",
            "Battery SOC",
            "EoiBattery.SocErrorFlagsAndBalancing.state_of_charge",
        )
        .unit("%")
        .device_class("battery")
        .measurement(),
    );
    v.push(
        sensor(
            "battery_error_flags",
            "Battery Error Flags",
            "EoiBattery.SocErrorFlagsAndBalancing.error_flags",
        )
        .icon("mdi:alert-circle-outline")
        .diagnostic(),
    );
    v.push(
        sensor(
            "battery_balancing_status",
            "Battery Balancing Status",
            "EoiBattery.SocErrorFlagsAndBalancing.balancing_status",
        )
        .icon("mdi:scale-balance")
        .diagnostic(),
    );

    // Cell voltages 1..14
    for (idx, (group, inner_idx)) in [
        ("CellVoltages1_4", 0),
        ("CellVoltages1_4", 1),
        ("CellVoltages1_4", 2),
        ("CellVoltages1_4", 3),
        ("CellVoltages5_8", 0),
        ("CellVoltages5_8", 1),
        ("CellVoltages5_8", 2),
        ("CellVoltages5_8", 3),
        ("CellVoltages9_12", 0),
        ("CellVoltages9_12", 1),
        ("CellVoltages9_12", 2),
        ("CellVoltages9_12", 3),
        ("CellVoltages13_14PackAndStack", 0),
        ("CellVoltages13_14PackAndStack", 1),
    ]
    .iter()
    .enumerate()
    {
        let cell = idx + 1;
        v.push(
            sensor(
                format!("battery_cell_{cell}_voltage"),
                format!("Battery Cell {cell} Voltage"),
                &format!("EoiBattery.{group}.cell_voltage[{inner_idx}]"),
            )
            .voltage()
            .display_precision(3),
        );
    }

    // Pack / stack voltages
    v.push(
        sensor(
            "battery_pack_voltage",
            "Battery Pack Voltage",
            "EoiBattery.CellVoltages13_14PackAndStack.pack_voltage",
        )
        .voltage(),
    );
    v.push(
        sensor(
            "battery_stack_voltage",
            "Battery Stack Voltage",
            "EoiBattery.CellVoltages13_14PackAndStack.stack_voltage",
        )
        .voltage(),
    );

    // Temperatures
    for i in 0..4 {
        let n = i + 1;
        v.push(
            sensor(
                format!("battery_temperature_{n}"),
                format!("Battery Temperature {n}"),
                &format!("EoiBattery.TemperaturesAndStates.temperatures[{i}]"),
            )
            .temperature(),
        );
    }
    v.push(
        sensor(
            "battery_ic_temperature",
            "Battery IC Temperature",
            "EoiBattery.TemperaturesAndStates.ic_temperature",
        )
        .temperature()
        .diagnostic(),
    );

    // States
    v.push(
        sensor(
            "battery_state",
            "Battery State",
            "EoiBattery.TemperaturesAndStates.battery_state",
        )
        .icon("mdi:battery-heart-variant"),
    );
    v.push(
        sensor(
            "battery_charge_state",
            "Battery Charge State",
            "EoiBattery.TemperaturesAndStates.charge_state",
        )
        .icon("mdi:battery-arrow-up"),
    );
    v.push(
        sensor(
            "battery_discharge_state",
            "Battery Discharge State",
            "EoiBattery.TemperaturesAndStates.discharge_state",
        )
        .icon("mdi:battery-arrow-down"),
    );

    // Uptime
    v.push(
        sensor(
            "battery_uptime_ms",
            "Battery Uptime",
            "EoiBattery.BatteryUptime.uptime_ms",
        )
        .unit("ms")
        .device_class("duration")
        .diagnostic(),
    );

    // Derived powers (computed in derived.rs)
    v.push(
        sensor(
            "battery_pack_power",
            "Battery Pack Power",
            "EoiBattery.PackAndPerriCurrent.pack_power",
        )
        .power(),
    );
    v.push(
        sensor(
            "battery_perri_power",
            "Battery Peripheral Power",
            "EoiBattery.PackAndPerriCurrent.perri_power",
        )
        .power(),
    );
    v.push(
        sensor(
            "battery_charge_power",
            "Battery Charge Power",
            "EoiBattery.ChargeAndDischargeCurrent.charge_power",
        )
        .power(),
    );
    v.push(
        sensor(
            "battery_discharge_power",
            "Battery Discharge Power",
            "EoiBattery.ChargeAndDischargeCurrent.discharge_power",
        )
        .power(),
    );
    v.push(
        sensor(
            "battery_stack_power",
            "Battery Stack Power",
            "EoiBattery.CellVoltages13_14PackAndStack.stack_power",
        )
        .power(),
    );
}

fn add_vesc(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, VESC);
    v.push(
        sensor("vesc_rpm", "VESC RPM", "Vesc.StatusMessage1.rpm")
            .unit("rpm")
            .measurement()
            .icon("mdi:engine"),
    );
    v.push(
        sensor(
            "vesc_total_current",
            "VESC Total Current",
            "Vesc.StatusMessage1.total_current",
        )
        .current(),
    );
    v.push(
        sensor(
            "vesc_duty_cycle",
            "VESC Duty Cycle",
            "Vesc.StatusMessage1.duty_cycle",
        )
        .unit("%")
        .measurement(),
    );

    v.push(
        sensor(
            "vesc_amp_hours_used",
            "VESC Amp Hours Used",
            "Vesc.StatusMessage2.amp_hours_used",
        )
        .charge_ah(),
    );
    v.push(
        sensor(
            "vesc_amp_hours_generated",
            "VESC Amp Hours Generated",
            "Vesc.StatusMessage2.amp_hours_generated",
        )
        .charge_ah(),
    );

    v.push(
        sensor(
            "vesc_watt_hours_used",
            "VESC Watt Hours Used",
            "Vesc.StatusMessage3.watt_hours_used",
        )
        .energy_wh(),
    );
    v.push(
        sensor(
            "vesc_watt_hours_generated",
            "VESC Watt Hours Generated",
            "Vesc.StatusMessage3.watt_hours_generated",
        )
        .energy_wh(),
    );

    v.push(
        sensor(
            "vesc_fet_temp",
            "VESC FET Temperature",
            "Vesc.StatusMessage4.fet_temp",
        )
        .temperature(),
    );
    v.push(
        sensor(
            "vesc_motor_temp",
            "Motor Temperature",
            "Vesc.StatusMessage4.motor_temp",
        )
        .temperature(),
    );
    v.push(
        sensor(
            "vesc_input_current",
            "VESC Input Current",
            "Vesc.StatusMessage4.total_input_current",
        )
        .current(),
    );
    v.push(
        sensor(
            "vesc_pid_position",
            "VESC PID Position",
            "Vesc.StatusMessage4.current_pid_position",
        )
        .measurement()
        .diagnostic(),
    );

    v.push(
        sensor(
            "vesc_input_voltage",
            "VESC Input Voltage",
            "Vesc.StatusMessage5.input_voltage",
        )
        .voltage(),
    );
    v.push(
        sensor(
            "vesc_tachometer",
            "VESC Tachometer",
            "Vesc.StatusMessage5.tachometer",
        )
        .total_increasing()
        .diagnostic()
        .icon("mdi:counter"),
    );

    // Derived (computed in derived.rs)
    v.push(
        sensor(
            "vesc_total_input_power",
            "VESC Total Input Power",
            "Vesc.StatusMessage4.total_input_power",
        )
        .power(),
    );
}

fn add_throttle(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, THROTTLE);
    v.push(
        sensor(
            "throttle_to_vesc_duty_cycle",
            "Throttle → Duty Cycle",
            "Throttle.ToVescDutyCycle",
        )
        .unit("%")
        .measurement(),
    );
    v.push(
        sensor(
            "throttle_to_vesc_current",
            "Throttle → Current",
            "Throttle.ToVescCurrent",
        )
        .current(),
    );
    v.push(
        sensor(
            "throttle_to_vesc_current_relative",
            "Throttle → Current Relative",
            "Throttle.ToVescCurrentRelative",
        )
        .unit("%")
        .measurement(),
    );
    v.push(
        sensor(
            "throttle_to_vesc_rpm",
            "Throttle → RPM",
            "Throttle.ToVescRpm",
        )
        .unit("rpm")
        .measurement(),
    );

    v.push(
        sensor(
            "throttle_status_value",
            "Throttle Position",
            "Throttle.Status.value",
        )
        .unit("%")
        .measurement()
        .icon("mdi:speedometer"),
    );
    v.push(
        sensor(
            "throttle_raw_angle",
            "Throttle Raw Angle",
            "Throttle.Status.raw_angle",
        )
        .measurement()
        .diagnostic(),
    );
    v.push(
        sensor(
            "throttle_raw_deadmen",
            "Throttle Raw Deadmen",
            "Throttle.Status.raw_deadmen",
        )
        .measurement()
        .diagnostic(),
    );
    v.push(
        sensor("throttle_gain", "Throttle Gain", "Throttle.Status.gain")
            .measurement()
            .diagnostic(),
    );

    v.push(
        sensor(
            "throttle_error_twi",
            "Throttle TWI Error",
            "Throttle.Status.error.twi",
        )
        .diagnostic()
        .icon("mdi:alert-circle-outline"),
    );
    v.push(
        sensor(
            "throttle_error_no_eeprom",
            "Throttle No EEPROM",
            "Throttle.Status.error.no_eeprom",
        )
        .diagnostic(),
    );
    v.push(
        sensor(
            "throttle_error_gain_clipping",
            "Throttle Gain Clipping",
            "Throttle.Status.error.gain_clipping",
        )
        .diagnostic(),
    );
    v.push(
        sensor(
            "throttle_error_gain_invalid",
            "Throttle Gain Invalid",
            "Throttle.Status.error.gain_invalid",
        )
        .diagnostic(),
    );
    v.push(
        binary_sensor(
            "throttle_error_deadman_missing",
            "Throttle Deadman Missing",
            "Throttle.Status.error.deadman_missing",
        )
        .device_class("problem")
        .diagnostic(),
    );
    v.push(
        sensor(
            "throttle_error_impedance_high",
            "Throttle Impedance High",
            "Throttle.Status.error.impedance_high",
        )
        .diagnostic(),
    );

    v.push(
        sensor(
            "throttle_control_type",
            "Throttle Control Type",
            "Throttle.Config.control_type",
        )
        .diagnostic()
        .icon("mdi:cog-outline"),
    );
    v.push(
        sensor(
            "throttle_active_control_type",
            "Throttle Active Control Type",
            "Throttle.ActiveControlType",
        )
        .diagnostic()
        .icon("mdi:cog-sync-outline"),
    );
    v.push(
        sensor(
            "throttle_lever_forward",
            "Throttle Lever Forward",
            "Throttle.Config.lever_forward",
        )
        .measurement()
        .diagnostic(),
    );
    v.push(
        sensor(
            "throttle_lever_backward",
            "Throttle Lever Backward",
            "Throttle.Config.lever_backward",
        )
        .measurement()
        .diagnostic(),
    );
}

fn add_rudder(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, RUDDER);
    v.push(
        sensor(
            "rudder_setpoint",
            "Rudder Setpoint",
            "RudderController.Servo.Setpoint",
        )
        .measurement()
        .icon("mdi:ship-wheel"),
    );
    v.push(
        sensor(
            "rudder_servo_state",
            "Rudder Servo State",
            "RudderController.Servo.Status.state",
        )
        .diagnostic()
        .icon("mdi:ship-wheel"),
    );
    v.push(
        sensor(
            "rudder_status_setpoint",
            "Rudder Status Setpoint",
            "RudderController.Servo.Status.setpoint",
        )
        .measurement()
        .icon("mdi:ship-wheel"),
    );
    v.push(
        sensor(
            "rudder_status_actual_position",
            "Rudder Status Actual Position",
            "RudderController.Servo.Status.actual_position",
        )
        .measurement()
        .icon("mdi:ship-wheel"),
    );
    v.push(
        sensor(
            "rudder_status_fault_cause",
            "Rudder Status Fault Cause",
            "RudderController.Servo.Status.fault_cause",
        )
        .diagnostic()
        .icon("mdi:console"),
    );
    v.push(
        sensor(
            "rudder_command",
            "Rudder Command",
            "RudderController.Servo.Command",
        )
        .diagnostic()
        .icon("mdi:console"),
    );
    v.push(
        sensor(
            "rudder_controller_temperature",
            "Rudder Controller Temperature",
            "Temperature.RudderController",
        )
        .temperature(),
    );
    v.push(
        sensor(
            "rudder_cooling_pump_status",
            "Rudder Cooling Pump Status",
            "RudderController.CoolingPumpStatus",
        )
        .diagnostic()
        .icon("mdi:pump"),
    );
    v.push(
        sensor(
            "rudder_steering_angle",
            "Rudder Steering Angle",
            "RudderController.SteeringAngle.angle",
        )
        .unit("°")
        .measurement()
        .icon("mdi:ship-wheel"),
    );
    v.push(
        sensor(
            "rudder_steering_angle_raw_adc",
            "Rudder Steering Angle Raw ADC",
            "RudderController.SteeringAngle.raw_adc",
        )
        .measurement()
        .diagnostic(),
    );
    for (variant, label, object_suffix) in [
        ("FlowSensorIn", "In", "in"),
        ("FlowSensorOut", "Out", "out"),
    ] {
        v.push(
            sensor(
                format!("rudder_flow_sensor_{object_suffix}_rate"),
                format!("Rudder Flow Sensor {label} Rate"),
                &format!("RudderController.{variant}.flow_rate"),
            )
            .unit("mL/min")
            .measurement()
            .icon("mdi:water-pump"),
        );
        v.push(
            sensor(
                format!("rudder_flow_sensor_{object_suffix}_temperature"),
                format!("Rudder Flow Sensor {label} Temperature"),
                &format!("RudderController.{variant}.temperature"),
            )
            .measurement()
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("rudder_flow_sensor_{object_suffix}_raw_pulses"),
                format!("Rudder Flow Sensor {label} Raw Pulses"),
                &format!("RudderController.{variant}.raw_pulses"),
            )
            .measurement()
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("rudder_flow_sensor_{object_suffix}_raw_adc"),
                format!("Rudder Flow Sensor {label} Raw ADC"),
                &format!("RudderController.{variant}.raw_adc"),
            )
            .measurement()
            .diagnostic(),
        );
    }
    // No motor temperature here: the rudder controller's 0x217 is retired, see RETIRED.
    // Motor temperature is the standalone node's, on the MOTOR device below.
}

/// The standalone motor NTC node, `0x219`. Its own device, since it is its own board
/// rather than a reading the rudder controller or the VESC happens to carry.
fn add_motor(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, MOTOR);
    v.push(
        sensor(
            "motor_temperature",
            "Motor Temperature",
            "Temperature.MotorNtc.temperature",
        )
        .temperature(),
    );
    for (object_id, name, field, problem) in [
        (
            "motor_ntc_sensor_open",
            "Motor NTC Sensor Open",
            "sensor_open",
            true,
        ),
        (
            "motor_ntc_sensor_short",
            "Motor NTC Sensor Short",
            "sensor_short",
            true,
        ),
        (
            "motor_ntc_out_of_range",
            "Motor NTC Out Of Range",
            "out_of_range",
            true,
        ),
        // Not a problem: it clears itself a few seconds after the node powers up.
        (
            "motor_ntc_settling",
            "Motor NTC Settling",
            "settling",
            false,
        ),
        (
            "motor_ntc_acquisition_error",
            "Motor NTC Acquisition Error",
            "acquisition_error",
            true,
        ),
        (
            "motor_ntc_previous_tx_failed",
            "Motor NTC Previous Tx Failed",
            "previous_tx_failed",
            true,
        ),
    ] {
        let entity = binary_sensor(
            object_id,
            name,
            &format!("Temperature.MotorNtc.status.{field}"),
        )
        .diagnostic();
        v.push(if problem {
            entity.device_class("problem")
        } else {
            entity
        });
    }
    v.push(
        sensor(
            "motor_ntc_frame_counter",
            "Motor NTC Frame Counter",
            "Temperature.MotorNtc.frame_counter",
        )
        .diagnostic()
        .icon("mdi:counter"),
    );
}

fn add_height_sensors(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, HEIGHT_SENSORS);
    for (variant, label, disabled_by_default) in [
        ("FrontLeft", "Front Left", false),
        ("FrontRight", "Front Right", false),
        ("Reserved1", "Height Sensor 3", true),
        ("Reserved2", "Height Sensor 4", true),
    ] {
        let id = variant.to_ascii_lowercase();
        let value_entity = sensor(
            format!("height_{id}_value"),
            format!("Height {label}"),
            &format!("HeightSensors.{variant}.value"),
        )
        .measurement()
        .icon("mdi:ruler");
        let state_entity = sensor(
            format!("height_{id}_state"),
            format!("Height {label} State"),
            &format!("HeightSensors.{variant}.state"),
        )
        .diagnostic()
        .icon("mdi:state-machine");
        if disabled_by_default {
            v.push(value_entity.disabled());
            v.push(state_entity.disabled());
        } else {
            v.push(value_entity);
            v.push(state_entity);
        }
    }
    v.push(
        sensor(
            "height_sensors_controller_temperature",
            "Height Sensors Controller Temperature",
            "Temperature.HeightSensorsController",
        )
        .temperature(),
    );
}

fn add_gnss(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, GNSS);
    v.push(
        sensor("gnss_fix", "GNSS Fix", "Gnss.GnssStatus.fix")
            .diagnostic()
            .icon("mdi:crosshairs-gps"),
    );
    v.push(
        sensor(
            "gnss_sats",
            "GNSS Satellites Visible",
            "Gnss.GnssStatus.sats",
        )
        .diagnostic()
        .icon("mdi:satellite-variant"),
    );
    v.push(
        sensor(
            "gnss_sats_used",
            "GNSS Satellites Used",
            "Gnss.GnssStatus.sats_used",
        )
        .diagnostic()
        .icon("mdi:satellite-variant"),
    );

    v.push(
        sensor("gnss_speed", "GNSS Speed", "Gnss.GnssSpeedAndHeading[0]")
            .unit("km/h")
            .device_class("speed")
            .measurement(),
    );
    v.push(
        sensor(
            "gnss_heading",
            "GNSS Heading",
            "Gnss.GnssSpeedAndHeading[1]",
        )
        .unit("°")
        .measurement()
        .icon("mdi:compass"),
    );

    v.push(
        sensor("gnss_latitude", "GNSS Latitude", "Gnss.GnssLatitude")
            .measurement()
            .icon("mdi:latitude"),
    );
    v.push(
        sensor("gnss_longitude", "GNSS Longitude", "Gnss.GnssLongitude")
            .measurement()
            .icon("mdi:longitude"),
    );

    v.push(
        sensor("gnss_year", "GNSS Year", "Gnss.GnssDateTime.year")
            .diagnostic()
            .disabled(),
    );
    v.push(
        sensor("gnss_month", "GNSS Month", "Gnss.GnssDateTime.month")
            .diagnostic()
            .disabled(),
    );
    v.push(
        sensor("gnss_day", "GNSS Day", "Gnss.GnssDateTime.day")
            .diagnostic()
            .disabled(),
    );
    v.push(
        sensor("gnss_hours", "GNSS Hours", "Gnss.GnssDateTime.hours")
            .diagnostic()
            .disabled(),
    );
    v.push(
        sensor("gnss_minutes", "GNSS Minutes", "Gnss.GnssDateTime.minutes")
            .diagnostic()
            .disabled(),
    );
    v.push(
        sensor("gnss_seconds", "GNSS Seconds", "Gnss.GnssDateTime.seconds")
            .diagnostic()
            .disabled(),
    );
}

fn add_mppt(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, MPPT);
    for node in 0..8u8 {
        let root = format!("Mppt.Id{node}");

        // 4 channels × (Power + State)
        for ch in 0..4u8 {
            let ch_root = format!("{root}.Channel{ch}");
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_voltage_in"),
                    format!("MPPT {node} Ch{ch} Voltage In"),
                    &format!("{ch_root}.Power.voltage_in"),
                )
                .voltage(),
            );
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_current_in"),
                    format!("MPPT {node} Ch{ch} Current In"),
                    &format!("{ch_root}.Power.current_in"),
                )
                .current(),
            );
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_power_in"),
                    format!("MPPT {node} Ch{ch} Power In"),
                    &format!("{ch_root}.Power.power_in"),
                )
                .power(),
            );
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_duty_cycle"),
                    format!("MPPT {node} Ch{ch} Duty Cycle"),
                    &format!("{ch_root}.State.duty_cycle"),
                )
                .measurement()
                .diagnostic(),
            );
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_algorithm"),
                    format!("MPPT {node} Ch{ch} Algorithm"),
                    &format!("{ch_root}.State.algorithm"),
                )
                .diagnostic(),
            );
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_algorithm_state"),
                    format!("MPPT {node} Ch{ch} Algorithm State"),
                    &format!("{ch_root}.State.algorithm_state"),
                )
                .diagnostic(),
            );
            v.push(
                sensor(
                    format!("mppt{node}_ch{ch}_channel_active"),
                    format!("MPPT {node} Ch{ch} Active"),
                    &format!("{ch_root}.State.channel_active"),
                )
                .diagnostic(),
            );
        }

        v.push(
            sensor(
                format!("mppt{node}_voltage_out"),
                format!("MPPT {node} Output Voltage"),
                &format!("{root}.Power.voltage_out"),
            )
            .voltage(),
        );
        v.push(
            sensor(
                format!("mppt{node}_current_out"),
                format!("MPPT {node} Output Current"),
                &format!("{root}.Power.current_out"),
            )
            .current(),
        );
        v.push(
            sensor(
                format!("mppt{node}_power_out"),
                format!("MPPT {node} Output Power"),
                &format!("{root}.Power.power_out"),
            )
            .power(),
        );

        v.push(
            sensor(
                format!("mppt{node}_voltage_out_switch"),
                format!("MPPT {node} Output Switch Voltage"),
                &format!("{root}.Status.voltage_out_switch"),
            )
            .voltage()
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("mppt{node}_temperature"),
                format!("MPPT {node} Temperature"),
                &format!("{root}.Status.temperature"),
            )
            .temperature(),
        );
        v.push(
            sensor(
                format!("mppt{node}_state"),
                format!("MPPT {node} State"),
                &format!("{root}.Status.state"),
            )
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("mppt{node}_pwm_enabled"),
                format!("MPPT {node} PWM Enabled"),
                &format!("{root}.Status.pwm_enabled"),
            )
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("mppt{node}_switch_on"),
                format!("MPPT {node} Switch On"),
                &format!("{root}.Status.switch_on"),
            )
            .diagnostic(),
        );
    }
}

fn add_gan_mppt(v: &mut Vec<HaEntity>) {
    let mut v = scoped(v, GAN_MPPT);
    for node in 0..16u8 {
        let root = format!("GanMppt.Id{node}");

        v.push(
            sensor(
                format!("gan_mppt{node}_input_voltage"),
                format!("GaN MPPT {node} Input Voltage"),
                &format!("{root}.Power.input_voltage"),
            )
            .voltage(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_input_current"),
                format!("GaN MPPT {node} Input Current"),
                &format!("{root}.Power.input_current"),
            )
            .current(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_output_voltage"),
                format!("GaN MPPT {node} Output Voltage"),
                &format!("{root}.Power.output_voltage"),
            )
            .voltage(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_output_current"),
                format!("GaN MPPT {node} Output Current"),
                &format!("{root}.Power.output_current"),
            )
            .current(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_input_power"),
                format!("GaN MPPT {node} Input Power"),
                &format!("{root}.Power.input_power"),
            )
            .power(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_output_power"),
                format!("GaN MPPT {node} Output Power"),
                &format!("{root}.Power.output_power"),
            )
            .power(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_efficiency"),
                format!("GaN MPPT {node} Efficiency"),
                &format!("{root}.Power.efficiency"),
            )
            .efficiency(),
        );

        v.push(
            sensor(
                format!("gan_mppt{node}_mode"),
                format!("GaN MPPT {node} Mode"),
                &format!("{root}.Status.mode"),
            )
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_fault"),
                format!("GaN MPPT {node} Fault"),
                &format!("{root}.Status.fault"),
            )
            .icon("mdi:alert-circle-outline"),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_enabled"),
                format!("GaN MPPT {node} Enabled"),
                &format!("{root}.Status.enabled"),
            )
            .diagnostic(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_board_temp"),
                format!("GaN MPPT {node} Board Temperature"),
                &format!("{root}.Status.board_temp"),
            )
            .temperature(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_heat_sink_temp"),
                format!("GaN MPPT {node} Heat Sink Temperature"),
                &format!("{root}.Status.heat_sink_temp"),
            )
            .temperature(),
        );

        v.push(
            sensor(
                format!("gan_mppt{node}_sweep_index"),
                format!("GaN MPPT {node} Sweep Index"),
                &format!("{root}.SweepData.index"),
            )
            .diagnostic()
            .disabled(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_sweep_current"),
                format!("GaN MPPT {node} Sweep Current"),
                &format!("{root}.SweepData.current"),
            )
            .current()
            .diagnostic()
            .disabled(),
        );
        v.push(
            sensor(
                format!("gan_mppt{node}_sweep_voltage"),
                format!("GaN MPPT {node} Sweep Voltage"),
                &format!("{root}.SweepData.voltage"),
            )
            .voltage()
            .diagnostic()
            .disabled(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eoi_can_decoder::*;
    use json_patch::merge;
    use std::collections::BTreeSet;

    fn datalogger_fixture() -> Value {
        json!({
            "DataLogger": {
                "Uptime": { "System": 0, "Process": 0 },
                "CpuLoad1M": 0.0,
                "CpuTemp": 0.0,
                "MemoryUsage": 0,
                "WifiIp": "0.0.0.0"
            }
        })
    }

    fn fixture_samples() -> Vec<EoiCanData> {
        let height_status = || HeightSensorStatus {
            state: HeightSensorState::Operational,
            value: 0,
        };
        let mut v: Vec<EoiCanData> = vec![
            // Battery
            EoiCanData::EoiBattery(EoiBattery::PackAndPerriCurrent(PackAndPerriCurrent {
                pack_current: 0.0,
                perri_current: 0.0,
            })),
            EoiCanData::EoiBattery(EoiBattery::ChargeAndDischargeCurrent(
                ChargeAndDischargeCurrent {
                    discharge_current: 0.0,
                    charge_current: 0.0,
                },
            )),
            EoiCanData::EoiBattery(EoiBattery::SocErrorFlagsAndBalancing(
                SocErrorFlagsAndBalancing {
                    state_of_charge: 0.0,
                    error_flags: 0,
                    balancing_status: 0,
                },
            )),
            EoiCanData::EoiBattery(EoiBattery::CellVoltages1_4(FourCellVoltages {
                cell_voltage: [0.0; 4],
            })),
            EoiCanData::EoiBattery(EoiBattery::CellVoltages5_8(FourCellVoltages {
                cell_voltage: [0.0; 4],
            })),
            EoiCanData::EoiBattery(EoiBattery::CellVoltages9_12(FourCellVoltages {
                cell_voltage: [0.0; 4],
            })),
            EoiCanData::EoiBattery(EoiBattery::CellVoltages13_14PackAndStack(
                CellVoltages13_14PackAndStack {
                    cell_voltage: [0.0; 2],
                    pack_voltage: 0.0,
                    stack_voltage: 0.0,
                },
            )),
            EoiCanData::EoiBattery(EoiBattery::TemperaturesAndStates(TemperaturesAndStates {
                temperatures: [0; 4],
                ic_temperature: 0,
                battery_state: BatteryState::Idle,
                charge_state: ChargeState::Idle,
                discharge_state: DischargeState::Idle,
            })),
            EoiCanData::EoiBattery(EoiBattery::BatteryUptime(BatteryUptime { uptime_ms: 0 })),
            // Vesc
            EoiCanData::Vesc(VescData::StatusMessage1 {
                rpm: 0,
                total_current: 0.0,
                duty_cycle: 0.0,
            }),
            EoiCanData::Vesc(VescData::StatusMessage2 {
                amp_hours_used: 0.0,
                amp_hours_generated: 0.0,
            }),
            EoiCanData::Vesc(VescData::StatusMessage3 {
                watt_hours_used: 0.0,
                watt_hours_generated: 0.0,
            }),
            EoiCanData::Vesc(VescData::StatusMessage4 {
                fet_temp: 0.0,
                motor_temp: 0.0,
                total_input_current: 0.0,
                current_pid_position: 0.0,
            }),
            EoiCanData::Vesc(VescData::StatusMessage5 {
                input_voltage: 0.0,
                tachometer: 0,
            }),
            // Throttle
            EoiCanData::Throttle(ThrottleData::ToVescDutyCycle(0.0)),
            EoiCanData::Throttle(ThrottleData::ToVescCurrent(0.0)),
            EoiCanData::Throttle(ThrottleData::ToVescCurrentRelative(0.0)),
            EoiCanData::Throttle(ThrottleData::ToVescRpm(0.0)),
            EoiCanData::Throttle(ThrottleData::Status(ThrottleStatus {
                value: 0.0,
                raw_angle: 0,
                raw_deadmen: 0,
                gain: 0,
                error: ThrottleErrors::default(),
            })),
            EoiCanData::Throttle(ThrottleData::Config(ThrottleConfig {
                control_type: ThrottleControlType::DutyCycle,
                lever_forward: 0,
                lever_backward: 0,
            })),
            // Rudder
            EoiCanData::RudderController(RudderControllerData::Servo(ServoData::Setpoint(0))),
            EoiCanData::RudderController(RudderControllerData::Servo(ServoData::Status(
                ServoStatus {
                    state: ServoState::Operational,
                    setpoint: 0,
                    actual_position: 0,
                    fault_cause: ServoFaultCause::None,
                },
            ))),
            EoiCanData::RudderController(RudderControllerData::Servo(ServoData::Command(
                ServoRudderCommand::Initialize,
            ))),
            EoiCanData::RudderController(RudderControllerData::CoolingPumpStatus(
                CoolingPumpStatus::Ok,
            )),
            EoiCanData::RudderController(RudderControllerData::SteeringAngle(SteeringAngle {
                angle: 0,
                raw_adc: 0,
            })),
            EoiCanData::RudderController(RudderControllerData::FlowSensorIn(FlowSensor {
                flow_rate: 0,
                temperature: Some(0.0),
                raw_pulses: 0,
                raw_adc: 0,
            })),
            EoiCanData::RudderController(RudderControllerData::FlowSensorOut(FlowSensor {
                flow_rate: 0,
                temperature: Some(0.0),
                raw_pulses: 0,
                raw_adc: 0,
            })),
            // No RudderController::MotorTemperature sample on purpose. The decoder still
            // parses 0x217 so archived logs keep flattening, but nothing transmits it any
            // more and it has no HA entity -- listing it here would fail the leaf check.
            // Height sensors
            EoiCanData::HeightSensors(HeightSensorData::FrontLeft(height_status())),
            EoiCanData::HeightSensors(HeightSensorData::FrontRight(height_status())),
            EoiCanData::HeightSensors(HeightSensorData::Reserved1(height_status())),
            EoiCanData::HeightSensors(HeightSensorData::Reserved2(height_status())),
            // Temperature
            EoiCanData::Temperature(TemperatureData::HeightSensorsController(0.0)),
            EoiCanData::Temperature(TemperatureData::RudderController(0.0)),
            EoiCanData::Temperature(TemperatureData::MotorNtc(MotorNtc {
                temperature: Some(0.0),
                status: MotorNtcStatus::default(),
                frame_counter: Some(0),
            })),
            // GNSS
            EoiCanData::Gnss(GnssData::GnssStatus(GnssStatus {
                fix: 0,
                sats: 0,
                sats_used: 0,
            })),
            EoiCanData::Gnss(GnssData::GnssSpeedAndHeading(0.0, 0.0)),
            EoiCanData::Gnss(GnssData::GnssLatitude(0.0)),
            EoiCanData::Gnss(GnssData::GnssLongitude(0.0)),
            EoiCanData::Gnss(GnssData::GnssDateTime(GnssDateTime {
                year: 0,
                month: 0,
                day: 0,
                hours: 0,
                minutes: 0,
                seconds: 0,
            })),
        ];

        // MPPT nodes 0..8
        for node in 0..8u8 {
            let mk = |info: MpptInfo| MpptData::from_node_id(node, info).unwrap();
            for ch in 0u8..4 {
                let channel_power = |p: MpptChannelPower| match ch {
                    0 => MpptInfo::Channel0(MpptChannel::Power(p)),
                    1 => MpptInfo::Channel1(MpptChannel::Power(p)),
                    2 => MpptInfo::Channel2(MpptChannel::Power(p)),
                    _ => MpptInfo::Channel3(MpptChannel::Power(p)),
                };
                let channel_state = |s: MpptChannelState| match ch {
                    0 => MpptInfo::Channel0(MpptChannel::State(s)),
                    1 => MpptInfo::Channel1(MpptChannel::State(s)),
                    2 => MpptInfo::Channel2(MpptChannel::State(s)),
                    _ => MpptInfo::Channel3(MpptChannel::State(s)),
                };
                v.push(EoiCanData::Mppt(mk(channel_power(MpptChannelPower {
                    voltage_in: 0.0,
                    current_in: 0.0,
                }))));
                v.push(EoiCanData::Mppt(mk(channel_state(MpptChannelState {
                    duty_cycle: 0,
                    algorithm: 0,
                    algorithm_state: 0,
                    channel_active: false,
                }))));
            }
            v.push(EoiCanData::Mppt(mk(MpptInfo::Power(MpptPower {
                voltage_out: 0.0,
                current_out: 0.0,
            }))));
            v.push(EoiCanData::Mppt(mk(MpptInfo::Status(MpptStatus {
                voltage_out_switch: 0.0,
                temperature: 0,
                state: 0,
                pwm_enabled: false,
                switch_on: false,
            }))));
        }

        // GaN MPPT nodes 0..16
        for node in 0..16u8 {
            let mk = |pkt: GanMpptPacket| GanMpptData::from_node_id(node, pkt).unwrap();
            v.push(EoiCanData::GanMppt(mk(GanMpptPacket::Power(
                GanMpptPower {
                    input_voltage: 2.0,
                    input_current: 1.0,
                    output_voltage: 1.0,
                    output_current: 1.0,
                },
            ))));
            v.push(EoiCanData::GanMppt(mk(GanMpptPacket::Status(
                GanMpptStatus {
                    mode: GanPhaseMode::None,
                    fault: GanPhaseFault::Ok,
                    enabled: false,
                    board_temp: 0,
                    heat_sink_temp: 0,
                },
            ))));
            v.push(EoiCanData::GanMppt(mk(GanMpptPacket::SweepData(
                GanMpptSweepData {
                    index: 0,
                    current: 0.0,
                    voltage: 0.0,
                },
            ))));
        }

        v
    }

    fn fixture_merged() -> Value {
        let mut root = datalogger_fixture();
        for s in fixture_samples() {
            let j = serde_json::to_value(&s).expect("serialize");
            merge(&mut root, &j);
        }
        crate::derived::apply_derived(&mut root);
        root
    }

    fn walk(v: &Value, path: String, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                for (k, val) in m {
                    let p = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    walk(val, p, out);
                }
            }
            Value::Array(a) => {
                for (i, item) in a.iter().enumerate() {
                    let p = format!("{path}[{i}]");
                    walk(item, p, out);
                }
            }
            _ => {
                out.insert(path);
            }
        }
    }

    fn json_leaf_paths() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        walk(&fixture_merged(), String::new(), &mut out);
        out
    }

    fn registry_paths() -> BTreeSet<String> {
        entities()
            .iter()
            .map(|e| e.path.clone().into_owned())
            .collect()
    }

    #[test]
    fn registry_matches_json_leaves() {
        let leaves = json_leaf_paths();
        let registry = registry_paths();

        let missing_in_registry: Vec<_> = leaves.difference(&registry).cloned().collect();
        let missing_in_json: Vec<_> = registry.difference(&leaves).cloned().collect();

        assert!(
            missing_in_registry.is_empty() && missing_in_json.is_empty(),
            "ha_discovery registry / JSON leaf mismatch.\n\n\
             Paths published to MQTT with no HaEntity (add a row to ha_discovery.rs):\n{:#?}\n\n\
             HaEntity value_templates referencing paths not present in JSON (stale registry row):\n{:#?}",
            missing_in_registry,
            missing_in_json
        );
    }

    /// A retired row publishes an empty retained payload, which deletes the entity in
    /// Home Assistant. If an object_id were live as well, whichever publish landed last
    /// would decide whether the entity exists -- so they must stay disjoint.
    #[test]
    fn retired_ids_are_not_live() {
        for (component, device, object_id) in RETIRED {
            assert!(
                !entities().iter().any(|e| e.component == *component
                    && e.device.id == device.id
                    && e.object_id == *object_id),
                "retired object_id is still in the registry: {}/{}/{}",
                component,
                device.id,
                object_id,
            );
        }
    }

    #[test]
    fn unique_object_ids() {
        let mut seen = BTreeSet::new();
        for e in entities() {
            assert!(
                seen.insert(e.object_id.clone().into_owned()),
                "duplicate object_id: {}",
                e.object_id,
            );
        }
    }

    #[test]
    fn device_block_has_via_device_for_non_hub() {
        for e in entities() {
            let cfg = build_config(&e);
            let device = cfg.get("device").and_then(|d| d.as_object()).unwrap();
            let ids = device
                .get("identifiers")
                .and_then(|v| v.as_array())
                .unwrap();
            assert_eq!(ids[0], json!(e.device.id));
            assert_eq!(device.get("name").unwrap(), &json!(e.device.name));
            if e.device.id == HUB.id {
                assert!(
                    device.get("via_device").is_none(),
                    "hub device should not link via_device"
                );
            } else {
                assert_eq!(
                    device.get("via_device").unwrap(),
                    &json!(HUB.id),
                    "non-hub device {} must set via_device to hub",
                    e.device.id,
                );
            }
        }
    }

    #[test]
    fn availability_template_chains_each_path_segment() {
        assert_eq!(
            availability_template_for("Mppt.Id0.Channel0.Power.power_in"),
            "{% if value_json.Mppt is defined and value_json.Mppt.Id0 is defined \
             and value_json.Mppt.Id0.Channel0 is defined \
             and value_json.Mppt.Id0.Channel0.Power is defined \
             and value_json.Mppt.Id0.Channel0.Power.power_in is defined \
             %}online{% else %}offline{% endif %}"
        );
    }

    #[test]
    fn every_entity_has_two_availability_sources() {
        for e in entities() {
            let cfg = build_config(&e);
            let avail = cfg
                .get("availability")
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("availability missing for {}", e.object_id));
            assert_eq!(
                avail.len(),
                2,
                "{} should have 2 availability sources (bridge + per-path)",
                e.object_id,
            );
            assert_eq!(cfg.get("availability_mode").unwrap(), &json!("all"));
        }
    }
}
