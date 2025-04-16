#![cfg_attr(not(test), no_std)]

pub mod can_frame;

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EoICanData {
    EoiBattery(EoiBattery),
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

pub fn parse_eoi_can_data(can_frame: &can_frame::CanFrame) -> Option<EoICanData> {
    let id = match can_frame.id {
        embedded_can::Id::Standard(id) => id.as_raw() as u32,
        embedded_can::Id::Extended(id) => id.as_raw(),
    };
    let data = &can_frame.data;

    match id {
        0x100 => Some(EoICanData::EoiBattery(EoiBattery::PackAndPerriCurrent(
            PackAndPerriCurrent {
                pack_current: bytes_to_f32(&data[0..4]),
                perri_current: bytes_to_f32(&data[4..8]),
            },
        ))),
        0x101 => Some(EoICanData::EoiBattery(
            EoiBattery::ChargeAndDischargeCurrent(ChargeAndDischargeCurrent {
                discharge_current: bytes_to_f32(&data[0..4]),
                charge_current: bytes_to_f32(&data[4..8]),
            }),
        )),
        0x102 => Some(EoICanData::EoiBattery(
            EoiBattery::SocErrorFlagsAndBalancing(SocErrorFlagsAndBalancing {
                state_of_charge: bytes_to_u16(&data[0..2]) as f32 / 100.0,
                error_flags: bytes_to_u32(&data[2..6]),
                balancing_status: bytes_to_u16(&data[6..8]),
            }),
        )),
        0x103 => Some(EoICanData::EoiBattery(EoiBattery::CellVoltages1_4(
            FourCellVoltages {
                cell_voltage: [
                    bytes_to_u16(&data[0..2]) as f32 / 1000.0,
                    bytes_to_u16(&data[2..4]) as f32 / 1000.0,
                    bytes_to_u16(&data[4..6]) as f32 / 1000.0,
                    bytes_to_u16(&data[6..8]) as f32 / 1000.0,
                ],
            },
        ))),
        0x104 => Some(EoICanData::EoiBattery(EoiBattery::CellVoltages5_8(
            FourCellVoltages {
                cell_voltage: [
                    bytes_to_u16(&data[0..2]) as f32 / 1000.0,
                    bytes_to_u16(&data[2..4]) as f32 / 1000.0,
                    bytes_to_u16(&data[4..6]) as f32 / 1000.0,
                    bytes_to_u16(&data[6..8]) as f32 / 1000.0,
                ],
            },
        ))),
        0x105 => Some(EoICanData::EoiBattery(EoiBattery::CellVoltages9_12(
            FourCellVoltages {
                cell_voltage: [
                    bytes_to_u16(&data[0..2]) as f32 / 1000.0,
                    bytes_to_u16(&data[2..4]) as f32 / 1000.0,
                    bytes_to_u16(&data[4..6]) as f32 / 1000.0,
                    bytes_to_u16(&data[6..8]) as f32 / 1000.0,
                ],
            },
        ))),
        0x106 => Some(EoICanData::EoiBattery(
            EoiBattery::CellVoltages13_14PackAndStack(CellVoltages13_14PackAndStack {
                cell_voltage: [
                    bytes_to_u16(&data[0..2]) as f32 / 1000.0,
                    bytes_to_u16(&data[2..4]) as f32 / 1000.0,
                ],
                pack_voltage: bytes_to_u16(&data[4..6]) as f32 / 1000.0,
                stack_voltage: bytes_to_u16(&data[6..8]) as f32 / 1000.0,
            }),
        )),
        0x107 => Some(EoICanData::EoiBattery(EoiBattery::TemperaturesAndStates(
            TemperaturesAndStates {
                temperatures: [data[0] as i8, data[1] as i8, data[2] as i8, data[3] as i8],
                ic_temperature: data[4] as i8,
                battery_state: data[5],
                charge_state: data[6],
                discharge_state: data[7],
            },
        ))),
        0x108 => Some(EoICanData::EoiBattery(EoiBattery::BatteryUptime(
            BatteryUptime {
                uptime_ms: bytes_to_u32(&data[0..4]),
            },
        ))),
        _ => None,
    }
}

fn bytes_to_u16(bytes: &[u8]) -> u16 {
    let arr: [u8; 2] = bytes.try_into().expect("Slice length must be 2");
    u16::from_le_bytes(arr)
}

fn bytes_to_u32(bytes: &[u8]) -> u32 {
    let arr: [u8; 4] = bytes.try_into().expect("Slice length must be 4");
    u32::from_le_bytes(arr)
}

fn bytes_to_f32(bytes: &[u8]) -> f32 {
    let arr: [u8; 4] = bytes.try_into().expect("Slice length must be 4");
    f32::from_le_bytes(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert2::assert;
    use embedded_can::StandardId;

    const PERRI_CURRENT: f32 = -0.2421;
    const DISCHARGE_CURRENT: f32 = 9.9765;
    const CHARGE_CURRENT: f32 = 17.5270;

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
        let data = if let EoICanData::EoiBattery(EoiBattery::PackAndPerriCurrent(data)) = data {
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
        let data = if let EoICanData::EoiBattery(EoiBattery::ChargeAndDischargeCurrent(data)) = data
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
        let data = if let EoICanData::EoiBattery(EoiBattery::SocErrorFlagsAndBalancing(data)) = data
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
        let data = if let EoICanData::EoiBattery(EoiBattery::CellVoltages1_4(data)) = data {
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
        let data = if let EoICanData::EoiBattery(EoiBattery::CellVoltages5_8(data)) = data {
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
        let data = if let EoICanData::EoiBattery(EoiBattery::CellVoltages9_12(data)) = data {
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
            if let EoICanData::EoiBattery(EoiBattery::CellVoltages13_14PackAndStack(data)) = data {
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
        let data = if let EoICanData::EoiBattery(EoiBattery::TemperaturesAndStates(data)) = data {
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
        let data = if let EoICanData::EoiBattery(EoiBattery::BatteryUptime(data)) = data {
            data
        } else {
            panic!("Unexpected data type");
        };
        assert!(data.uptime_ms == 992129132);
    }
}
