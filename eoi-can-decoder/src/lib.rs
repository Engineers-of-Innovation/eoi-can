#![cfg_attr(feature = "defmt", no_std)]

pub mod can_collector;
pub mod can_frame;

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EoiCanData {
    EoiBattery(EoiBattery),
    Vesc(VescData),
    Throttle(ThrottleData),
    Mppt(MpptData),
    Gnss(GnssData),
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GnssData {
    GnssStatus(GnssStatus),
    GnssSpeedAndHeading(f32, f32),
    GnssLatitude(f64),
    GnssLongitude(f64),
    GnssDateTime(GnssDateTime),
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct GnssStatus {
    pub fix: u8,
    pub sats: u8,
    pub sats_used: u8,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct GnssDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThrottleData {
    ToVescDutyCycle(f32),
    ToVescCurrent(f32),
    ToVescRpm(f32),
    Status(ThrottleStatus),
    Config(ThrottleConfig),
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ThrottleStatus {
    pub value: f32,
    pub raw_angle: i16,
    pub raw_deadmen: i16,
    pub gain: u8,
    pub error_any: bool,
    pub error_twi: u8,
    pub error_no_eeprom: bool,
    pub error_gain_clipping: bool,
    pub error_gain_invalid: bool,
    pub error_deadman_missing: bool,
    pub error_impedance_high: bool,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ThrottleConfig {
    pub control_type: ThrottleControlType,
    pub lever_forward: i16,
    pub lever_backward: i16,
}

#[derive(Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum ThrottleControlType {
    DutyCycle = 0,
    FilteredDutyCycle = 1,
    Current = 2,
    Rpm = 3,
    CurrentRelative = 4,
    #[default]
    Unknown = 255,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MpptData {
    pub mppt_id: u8,
    pub info: MpptInfo,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MpptInfo {
    MpptChannelPower(MpptChannelPower),
    MpptChannelState(MpptChannelState),
    MpptPower(MpptPower),
    MpptStatus(MpptStatus),
}
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MpptChannelPower {
    pub mppt_channel: u8,
    pub voltage_in: f32,
    pub current_in: f32,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MpptChannelState {
    pub mppt_channel: u8,
    pub duty_cycle: u16,
    pub algorithm: u8,
    pub algorithm_state: u8,
    pub channel_active: bool,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MpptPower {
    pub voltage_out: f32,
    pub current_out: f32,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MpptStatus {
    pub voltage_out_switch: f32,
    pub temperature: i16,
    pub state: u8,
    pub pwm_enabled: bool,
    pub switch_on: bool,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EoiBattery {
    PackAndPerriCurrent(PackAndPerriCurrent),
    ChargeAndDischargeCurrent(ChargeAndDischargeCurrent),
    SocErrorFlagsAndBalancing(SocErrorFlagsAndBalancing),
    CellVoltages1_4(FourCellVoltages),
    CellVoltages5_8(FourCellVoltages),
    CellVoltages9_12(FourCellVoltages),
    CellVoltages13_14PackAndStack(CellVoltages13_14PackAndStack),
    TemperaturesAndStates(TemperaturesAndStates),
    BatteryUptime(BatteryUptime),
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PackAndPerriCurrent {
    pub pack_current: f32,
    pub perri_current: f32,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChargeAndDischargeCurrent {
    pub discharge_current: f32,
    pub charge_current: f32,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SocErrorFlagsAndBalancing {
    pub state_of_charge: f32,  // u16 on CAN bus with a factor of 100
    pub error_flags: u32,      //TODO: use bitflags?!
    pub balancing_status: u16, //TODO: use bitflags?!
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FourCellVoltages {
    pub cell_voltage: [f32; 4], // u16 on CAN bus with a factor of 1000
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CellVoltages13_14PackAndStack {
    pub cell_voltage: [f32; 2], // u16 on CAN bus with a factor of 1000
    pub pack_voltage: f32,      // u16 on CAN bus with a factor of 1000
    pub stack_voltage: f32,     // u16 on CAN bus with a factor of 1000
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TemperaturesAndStates {
    pub temperatures: [i8; 4],
    pub ic_temperature: i8,
    pub battery_state: u8,   //TODO: define enum
    pub charge_state: u8,    //TODO: define enum
    pub discharge_state: u8, //TODO: define enum
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BatteryUptime {
    pub uptime_ms: u32,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VescData {
    StatusMessage1 {
        rpm: i32,
        total_current: f32,
        duty_cycle: f32,
    },
    StatusMessage2 {
        amp_hours_used: f32,
        amp_hours_generated: f32,
    },
    StatusMessage3 {
        watt_hours_used: f32,
        watt_hours_generated: f32,
    },
    StatusMessage4 {
        fet_temp: f32,
        motor_temp: f32,
        total_input_current: f32,
        current_pid_position: f32,
    },
    StatusMessage5 {
        input_voltage: f32,
        tachometer: i32,
    },
}

pub fn parse_eoi_can_data(can_frame: &can_frame::CanFrame) -> Option<EoiCanData> {
    let id = match can_frame.id {
        embedded_can::Id::Standard(id) => id.as_raw() as u32,
        embedded_can::Id::Extended(id) => id.as_raw(),
    };
    let data = &can_frame.data;

    const MPPT_MAX_DEVICES: u32 = 8;
    const MPPT_BASE_ADDRESS: u32 = 0x700;
    const MPPT_INFO_FIELDS: u32 = 16;
    const MPPT_STOP_ADDRESS: u32 = MPPT_BASE_ADDRESS + (MPPT_MAX_DEVICES * MPPT_INFO_FIELDS) - 1;

    match id {
        0x100 => Some(EoiCanData::EoiBattery(EoiBattery::PackAndPerriCurrent(
            PackAndPerriCurrent {
                pack_current: bytes_le_to_f32(data.get(0..4)?)?,
                perri_current: bytes_le_to_f32(data.get(4..8)?)?,
            },
        ))),
        0x101 => Some(EoiCanData::EoiBattery(
            EoiBattery::ChargeAndDischargeCurrent(ChargeAndDischargeCurrent {
                charge_current: bytes_le_to_f32(data.get(0..4)?)?,
                discharge_current: -1.0 * bytes_le_to_f32(data.get(4..8)?)?,
            }),
        )),
        0x102 => Some(EoiCanData::EoiBattery(
            EoiBattery::SocErrorFlagsAndBalancing(SocErrorFlagsAndBalancing {
                state_of_charge: bytes_le_to_u16(data.get(0..2)?)? as f32 / 100.0,
                error_flags: bytes_le_to_u32(data.get(2..6)?)?,
                balancing_status: bytes_le_to_u16(data.get(6..8)?)?,
            }),
        )),
        0x103 => Some(EoiCanData::EoiBattery(EoiBattery::CellVoltages1_4(
            FourCellVoltages {
                cell_voltage: [
                    bytes_le_to_u16(data.get(0..2)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(2..4)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(4..6)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(6..8)?)? as f32 / 1000.0,
                ],
            },
        ))),
        0x104 => Some(EoiCanData::EoiBattery(EoiBattery::CellVoltages5_8(
            FourCellVoltages {
                cell_voltage: [
                    bytes_le_to_u16(data.get(0..2)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(2..4)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(4..6)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(6..8)?)? as f32 / 1000.0,
                ],
            },
        ))),
        0x105 => Some(EoiCanData::EoiBattery(EoiBattery::CellVoltages9_12(
            FourCellVoltages {
                cell_voltage: [
                    bytes_le_to_u16(data.get(0..2)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(2..4)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(4..6)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(6..8)?)? as f32 / 1000.0,
                ],
            },
        ))),
        0x106 => Some(EoiCanData::EoiBattery(
            EoiBattery::CellVoltages13_14PackAndStack(CellVoltages13_14PackAndStack {
                cell_voltage: [
                    bytes_le_to_u16(data.get(0..2)?)? as f32 / 1000.0,
                    bytes_le_to_u16(data.get(2..4)?)? as f32 / 1000.0,
                ],
                pack_voltage: bytes_le_to_u16(data.get(4..6)?)? as f32 / 1000.0,
                stack_voltage: bytes_le_to_u16(data.get(6..8)?)? as f32 / 1000.0,
            }),
        )),
        0x107 => Some(EoiCanData::EoiBattery(EoiBattery::TemperaturesAndStates(
            TemperaturesAndStates {
                temperatures: [
                    *data.first()? as i8,
                    *data.get(1)? as i8,
                    *data.get(2)? as i8,
                    *data.get(3)? as i8,
                ],
                ic_temperature: *data.get(4)? as i8,
                battery_state: *data.get(5)?,
                charge_state: *data.get(6)?,
                discharge_state: *data.get(7)?,
            },
        ))),
        0x108 => Some(EoiCanData::EoiBattery(EoiBattery::BatteryUptime(
            BatteryUptime {
                uptime_ms: bytes_le_to_u32(data.get(0..4)?)?,
            },
        ))),

        0x200 => Some(EoiCanData::Gnss(GnssData::GnssStatus(GnssStatus {
            fix: *data.first()?,
            sats: *data.get(1)?,
            sats_used: *data.get(2)?,
        }))),
        0x201 => Some(EoiCanData::Gnss(GnssData::GnssSpeedAndHeading(
            bytes_le_to_f32(data.get(0..4)?)?,
            bytes_le_to_f32(data.get(4..8)?)?,
        ))),
        0x202 => Some(EoiCanData::Gnss(GnssData::GnssLatitude(bytes_le_to_f64(
            data.get(0..8)?,
        )?))),
        0x203 => Some(EoiCanData::Gnss(GnssData::GnssLongitude(bytes_le_to_f64(
            data.get(0..8)?,
        )?))),
        0x204 => Some(EoiCanData::Gnss(GnssData::GnssDateTime(GnssDateTime {
            year: bytes_le_to_u16(data.get(0..2)?)?,
            month: *data.get(2)?,
            day: *data.get(3)?,
            hours: *data.get(4)?,
            minutes: *data.get(5)?,
            seconds: *data.get(6)?,
        }))),

        MPPT_BASE_ADDRESS..MPPT_STOP_ADDRESS => {
            let mppt_id = ((id >> 4) & 0x7) as u8;
            let info_field = id as u8 & 0xF;
            let channel = info_field >> 1;
            match info_field {
                0 | 2 | 4 | 6 => {
                    let mppt_data = MpptData {
                        mppt_id,
                        info: MpptInfo::MpptChannelPower(MpptChannelPower {
                            mppt_channel: channel,
                            voltage_in: bytes_le_to_f32(data.get(0..4)?)?,
                            current_in: bytes_le_to_f32(data.get(4..8)?)?,
                        }),
                    };
                    Some(EoiCanData::Mppt(mppt_data))
                }
                1 | 3 | 5 | 7 => {
                    let mppt_data = MpptData {
                        mppt_id,
                        info: MpptInfo::MpptChannelState(MpptChannelState {
                            mppt_channel: channel,
                            duty_cycle: bytes_le_to_u16(data.get(0..2)?)?,
                            algorithm: *data.get(2)?,
                            algorithm_state: *data.get(3)?,
                            channel_active: *data.get(4)? != 0,
                        }),
                    };
                    Some(EoiCanData::Mppt(mppt_data))
                }
                8 => {
                    let mppt_data = MpptData {
                        mppt_id,
                        info: MpptInfo::MpptPower(MpptPower {
                            voltage_out: bytes_le_to_f32(data.get(0..4)?)?,
                            current_out: bytes_le_to_f32(data.get(4..8)?)?,
                        }),
                    };
                    Some(EoiCanData::Mppt(mppt_data))
                }
                9 => {
                    let mppt_data = MpptData {
                        mppt_id,
                        info: MpptInfo::MpptStatus(MpptStatus {
                            voltage_out_switch: bytes_le_to_f32(data.get(0..4)?)?,
                            temperature: bytes_le_to_i16(data.get(4..6)?)?,
                            state: *data.get(6)?,
                            pwm_enabled: *data.get(7)? & 0b1 != 0,
                            switch_on: *data.get(7)? & 0b10 != 0,
                        }),
                    };
                    Some(EoiCanData::Mppt(mppt_data))
                }
                _ => {
                    // Ignore other info fields
                    None
                }
            }
        }

        0x0909 => Some(EoiCanData::Vesc(VescData::StatusMessage1 {
            rpm: bytes_be_to_i32(data.get(0..4)?)?,
            total_current: bytes_be_to_i16(data.get(4..6)?)? as f32 / 10.0,
            duty_cycle: bytes_be_to_i16(data.get(6..8)?)? as f32 / 10.0,
        })),
        0x0E09 => Some(EoiCanData::Vesc(VescData::StatusMessage2 {
            amp_hours_used: bytes_be_to_u32(data.get(0..4)?)? as f32 / 10000.0,
            amp_hours_generated: bytes_be_to_u32(data.get(4..8)?)? as f32 / 10000.0,
        })),
        0x0F09 => Some(EoiCanData::Vesc(VescData::StatusMessage3 {
            watt_hours_used: bytes_be_to_u32(data.get(0..4)?)? as f32 / 10000.0,
            watt_hours_generated: bytes_be_to_u32(data.get(4..8)?)? as f32 / 10000.0,
        })),
        0x1009 => Some(EoiCanData::Vesc(VescData::StatusMessage4 {
            fet_temp: bytes_be_to_i16(data.get(0..2)?)? as f32 / 10.0,
            motor_temp: bytes_be_to_i16(data.get(2..4)?)? as f32 / 10.0,
            total_input_current: bytes_be_to_i16(data.get(4..6)?)? as f32 / 10.0,
            current_pid_position: bytes_be_to_i16(data.get(6..8)?)? as f32 / 50.0,
        })),
        0x1B09 => Some(EoiCanData::Vesc(VescData::StatusMessage5 {
            input_voltage: bytes_be_to_i16(data.get(4..6)?)? as f32 / 10.0,
            tachometer: bytes_be_to_i32(data.get(0..4)?)?,
        })),
        0x0009 => Some(EoiCanData::Throttle(ThrottleData::ToVescDutyCycle(
            bytes_be_to_i32(data.get(0..4)?)? as f32 / 1000.0,
        ))),
        0x0109 => Some(EoiCanData::Throttle(ThrottleData::ToVescCurrent(
            bytes_be_to_i32(data.get(0..4)?)? as f32 / 1000.0,
        ))),
        0x0309 => Some(EoiCanData::Throttle(ThrottleData::ToVescRpm(
            bytes_be_to_i32(data.get(0..4)?)? as f32 / 1000.0,
        ))),
        0x1337 | 0x0337 => match data.len() {
            8 => Some(EoiCanData::Throttle(ThrottleData::Status(ThrottleStatus {
                value: (bytes_be_to_i16(data.get(0..2)?)? as f32 / 512.0) * 100.0,
                raw_angle: bytes_be_to_i16(data.get(2..4)?)?,
                raw_deadmen: bytes_be_to_i16(data.get(4..6)?)?,
                gain: *data.get(6)?,
                error_any: *data.get(7)? != 0,
                error_twi: *data.get(7)? & 0b111,
                error_no_eeprom: *data.get(7)? & (1 << 3) != 0,
                error_gain_clipping: *data.get(7)? & (1 << 4) != 0,
                error_gain_invalid: *data.get(7)? & (1 << 5) != 0,
                error_deadman_missing: *data.get(7)? & (1 << 6) != 0,
                error_impedance_high: *data.get(7)? & (1 << 7) != 0,
            }))),
            6 => Some(EoiCanData::Throttle(ThrottleData::Config(ThrottleConfig {
                control_type: match *data.first()? {
                    0 => ThrottleControlType::DutyCycle,
                    1 => ThrottleControlType::FilteredDutyCycle,
                    2 => ThrottleControlType::Current,
                    3 => ThrottleControlType::Rpm,
                    4 => ThrottleControlType::CurrentRelative,
                    _ => ThrottleControlType::Unknown,
                },
                lever_forward: bytes_be_to_i16(data.get(2..4)?)?,
                lever_backward: bytes_be_to_i16(data.get(4..6)?)?,
            }))),
            _ => None,
        },
        _ => None,
    }
}

// Helper functions now return Option<T> instead of panicking

fn bytes_le_to_u16(bytes: &[u8]) -> Option<u16> {
    let arr: [u8; 2] = bytes.try_into().ok()?;
    Some(u16::from_le_bytes(arr))
}

fn bytes_be_to_i16(bytes: &[u8]) -> Option<i16> {
    let arr: [u8; 2] = bytes.try_into().ok()?;
    Some(i16::from_be_bytes(arr))
}

fn bytes_le_to_u32(bytes: &[u8]) -> Option<u32> {
    let arr: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_le_bytes(arr))
}

fn bytes_be_to_u32(bytes: &[u8]) -> Option<u32> {
    let arr: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_be_bytes(arr))
}

fn bytes_le_to_f32(bytes: &[u8]) -> Option<f32> {
    let arr: [u8; 4] = bytes.try_into().ok()?;
    Some(f32::from_le_bytes(arr))
}

fn bytes_le_to_f64(bytes: &[u8]) -> Option<f64> {
    let arr: [u8; 8] = bytes.try_into().ok()?;
    Some(f64::from_le_bytes(arr))
}

fn bytes_le_to_i16(bytes: &[u8]) -> Option<i16> {
    let arr: [u8; 2] = bytes.try_into().ok()?;
    Some(i16::from_le_bytes(arr))
}

fn bytes_be_to_i32(bytes: &[u8]) -> Option<i32> {
    let arr: [u8; 4] = bytes.try_into().ok()?;
    Some(i32::from_be_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert2::assert;
    use embedded_can::StandardId;

    const PERRI_CURRENT: f32 = -0.2421;
    const CHARGE_CURRENT: f32 = 9.9765;
    const DISCHARGE_CURRENT: f32 = -17.5270;

    // This sum seems a bit odd, but it is the result of the test data
    // TODO: investigate if this is a bug in the test data / battery
    const PACK_CURRENT: f32 = CHARGE_CURRENT + DISCHARGE_CURRENT + PERRI_CURRENT;

    #[test]
    fn pack_and_perri_current() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x100).unwrap()),
            &0x5817DA41EBF577BE_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::PackAndPerriCurrent(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!((data.pack_current - PACK_CURRENT).abs() < 0.0001);
        assert!((data.perri_current - PERRI_CURRENT).abs() < 0.0001);
    }

    #[test]
    fn charge_and_discharge_current() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x101).unwrap()),
            &0xE89F1F4150378C41_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::ChargeAndDischargeCurrent(data)) = data
        {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!((data.discharge_current - DISCHARGE_CURRENT).abs() < 0.0001);
        assert!((data.charge_current - CHARGE_CURRENT).abs() < 0.0001);
    }

    #[test]
    fn soc_error_flags_and_balancing() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x102).unwrap()),
            &0x2526000000000000_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::SocErrorFlagsAndBalancing(data)) = data
        {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.state_of_charge == 97.65);
        assert!(data.error_flags == 0);
        assert!(data.balancing_status == 0);
    }

    #[test]
    fn cell_voltages_1_4() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x103).unwrap()),
            &0x36102C102D103710_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::CellVoltages1_4(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.cell_voltage[0] == 4.150);
        assert!(data.cell_voltage[1] == 4.140);
        assert!(data.cell_voltage[2] == 4.141);
        assert!(data.cell_voltage[3] == 4.151);
    }

    #[test]
    fn cell_voltages_5_8() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x104).unwrap()),
            &0x34103A1030103410_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::CellVoltages5_8(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.cell_voltage[0] == 4.148);
        assert!(data.cell_voltage[1] == 4.154);
        assert!(data.cell_voltage[2] == 4.144);
        assert!(data.cell_voltage[3] == 4.148);
    }

    #[test]
    fn cell_voltages_9_12() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x105).unwrap()),
            &0x3810391038103410_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::CellVoltages9_12(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.cell_voltage[0] == 4.152);
        assert!(data.cell_voltage[1] == 4.153);
        assert!(data.cell_voltage[2] == 4.152);
        assert!(data.cell_voltage[3] == 4.148);
    }

    #[test]
    fn cell_voltages_13_14_pack_and_stack() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x106).unwrap()),
            &0x39103110C0DA0EE2_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data =
            if let EoiCanData::EoiBattery(EoiBattery::CellVoltages13_14PackAndStack(data)) = data {
                data
            } else {
                panic!("Unexpected data type");
            };
        assert!(data.cell_voltage[0] == 4.153);
        assert!(data.cell_voltage[1] == 4.145);
        assert!(data.pack_voltage == 56.0);
        assert!(data.stack_voltage == 57.87);
    }

    #[test]
    fn temperatures_and_states() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x107).unwrap()),
            &0x2424262836060303_u64.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::TemperaturesAndStates(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.temperatures[0] == 36);
        assert!(data.temperatures[1] == 36);
        assert!(data.temperatures[2] == 38);
        assert!(data.temperatures[3] == 40);
        assert!(data.ic_temperature == 54);
        assert!(data.battery_state == 6);
        assert!(data.charge_state == 3);
        assert!(data.discharge_state == 3);
    }

    #[test]
    fn battery_uptime() {
        let can_frame = can_frame::CanFrame::from_encoded(
            embedded_can::Id::Standard(StandardId::new(0x108).unwrap()),
            &0x6CB0223B_u32.to_be_bytes(),
        );

        let data = parse_eoi_can_data(&can_frame).unwrap();
        let data = if let EoiCanData::EoiBattery(EoiBattery::BatteryUptime(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.uptime_ms == 992129132);
    }
}
