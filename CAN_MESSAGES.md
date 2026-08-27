# CAN Messages

Any state byte value not listed maps to `Unknown` on the receiver side.

## Overview

| CAN ID | Message | Device |
| --- | --- | --- |
| 0x009 | ThrottleToVescDutyCycle | Throttle Controller |
| 0x010 | ServoRudderSetpoint | Rudder Controller |
| 0x011 | HeightSensorFrontLeft | Height Sensors |
| 0x012 | HeightSensorFrontRight | Height Sensors |
| 0x013 | HeightSensor (placement TBD) | Height Sensors |
| 0x014 | HeightSensor (placement TBD) | Height Sensors |
| 0x030 | BootloaderDiscovery | Bootloader (broadcast, all boards) |
| 0x031–0x033 | Bootloader cmd / resp / data | Bootloader (Rudder Controller) |
| 0x034–0x036 | Bootloader cmd / resp / data | Bootloader (Height Sensor Controller) |
| 0x037–0x039 | Bootloader cmd / resp / data | Bootloader (Dashboard) |
| 0x03A–0x03F | _unallocated_ | Bootloader (app types 0x04, 0x05) |
| 0x020 | ServoRudderStatus | Rudder Controller |
| 0x021 | ServoRudderCommand | Rudder Controller |
| 0x100 | PackAndPerriCurrent | Battery Management System |
| 0x101 | ChargeAndDischargeCurrent | Battery Management System |
| 0x102 | SocErrorFlagsAndBalancing | Battery Management System |
| 0x103 | CellVoltages1To4 | Battery Management System |
| 0x104 | CellVoltages5To8 | Battery Management System |
| 0x105 | CellVoltages9To12 | Battery Management System |
| 0x106 | CellVoltages13To14PackAndStack | Battery Management System |
| 0x107 | TemperaturesAndStates | Battery Management System |
| 0x108 | BatteryUptime | Battery Management System |
| 0x109 | ThrottleToVescCurrent | Throttle Controller |
| 0xA09 | ThrottleToVescCurrentRelative | Throttle Controller |
| 0x200 | GnssStatus | GNSS |
| 0x210 | TemperatureHeightSensorsController | Height Sensors |
| 0x211 | TemperatureRudderController | Rudder Controller |
| 0x212 | RudderControllerCoolingPumpStatus | Rudder Controller |
| 0x213 | SteeringAngle | Rudder Controller |
| 0x214 | SteeringAngleCalibration (reserved) | Rudder Controller |
| 0x215 | FlowSensorIn | Rudder Controller |
| 0x216 | FlowSensorOut | Rudder Controller |
| 0x217 | MotorTemperature (retired, do not reuse) | Rudder Controller |
| 0x219 | MotorNtc | Motor NTC Sensor |
| 0x201 | GnssSpeedAndHeading | GNSS |
| 0x202 | GnssLatitude | GNSS |
| 0x203 | GnssLongitude | GNSS |
| 0x204 | GnssDateTime | GNSS |
| 0x205 | DataLoggerWifiIp | Data Logger |
| 0x309 | ThrottleToVescRpm | Throttle Controller |
| 0x337 | ThrottleStatus / ThrottleConfig | Throttle Controller |
| 0x400–0x4FF | GanMppt\* | GaN MPPT Solar Controllers |
| 0x700–0x77F | Mppt\* | MPPT Solar Controllers |
| 0x909 | VescStatusMessage1 | VESC Motor Controller |
| 0xE09 | VescStatusMessage2 | VESC Motor Controller |
| 0xF09 | VescStatusMessage3 | VESC Motor Controller |
| 0x1009 | VescStatusMessage4 | VESC Motor Controller |
| 0x1337 | ThrottleStatus / ThrottleConfig | Throttle Controller |
| 0x1B09 | VescStatusMessage5 | VESC Motor Controller |

## Rudder Controller

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ServoRudderSetpoint | 0x010 | 2 | 0–1 | Setpoint | u16 | LE | 1000–2000. Out-of-range values are rejected by the rudder controller and do not feed its 2 s communication watchdog. |
| ServoRudderStatus | 0x020 | 6 | 0 | State | u8 enum | | 0=Uninitialized, 1=Operational, 2=Homing, 3=FailSafe, 4=Fault, 0xFF=Unknown |
| | | | 1–2 | Current setpoint | u16 | LE | 1000–2000 |
| | | | 3–4 | Actual position | u16 | LE | 1000–2000, in setpoint units. Sent every 100 ms. |
| | | | 5 | Fault cause | u8 enum | | 0=None, 1=StallDuringMove, 2=HomingTimeout, 3=DriverNoUartResponse, 4=DriverError |
| ServoRudderCommand | 0x021 | 1 | 0 | Command | u8 enum | | 0=Initialize. Starts (re-)homing from any state; required to leave FailSafe or Fault. |
| RudderControllerCoolingPumpStatus | 0x212 | 1 | 0 | Fault input level | u8 | | Raw PC5 level: 0=fault asserted (low), 1=ok (high). Sent every 1 s. |
| SteeringAngle | 0x213 | 4 | 0–1 | Angle | i16 | LE | -180 to +180 degrees. Sent every 100 ms. |
| | | | 2–3 | Raw ADC | u16 | LE | 0–4095 (12-bit). Linear mapping: 0=-180°, 4095=+180°. |
| SteeringAngleCalibration | 0x214 | TBD | TBD | TBD | TBD | | Reserved for steering angle calibration; format not yet defined. |
| FlowSensorIn | 0x215 | 8 | 0–1 | Flow rate | u16 | LE | mL/min. Sent every 1 s. Datasheet: 22.9 Hz at 1880 mL/min. |
| | | | 2–3 | Temperature | i16 | LE | Centidegrees Celsius. `i16::MIN` (`-32768`) means open/shorted NTC. |
| | | | 4–5 | Raw pulses | u16 | LE | Pulses counted in the last 1 s window. |
| | | | 6–7 | Raw ADC | u16 | LE | 12-bit NTC ADC code (0–4095). NTC = 50 kΩ at 25 °C, B=3950, top = 47 kΩ. |
| FlowSensorOut | 0x216 | 8 | 0–1 | Flow rate | u16 | LE | mL/min. Same scaling and cadence as `FlowSensorIn`. |
| | | | 2–3 | Temperature | i16 | LE | Centidegrees Celsius. `i16::MIN` means open/shorted NTC. |
| | | | 4–5 | Raw pulses | u16 | LE | Pulses counted in the last 1 s window. |
| | | | 6–7 | Raw ADC | u16 | LE | 12-bit NTC ADC code (0–4095). Same NTC part as `FlowSensorIn` (50 kΩ at 25 °C, B=3950, top = 47 kΩ), read on PA3 — the pin the retired 0x217 borrowed. |
| MotorTemperature | 0x217 | 4 | 0–1 | Temperature | i16 | LE | **Retired — no longer transmitted.** Motor temperature comes from `MotorNtc` (0x219). Was centidegrees Celsius, `i16::MIN` (`-32768`) for an open/shorted NTC, every 1 s, NTC = 10 kΩ at 25 °C, B=3950, top = 10 kΩ. The decoder still parses it so archived logs replay; the ID is reserved and must not be reused. |
| | | | 2–3 | Raw ADC | u16 | LE | 12-bit NTC ADC code (0–4095). |

## Height Sensors

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| HeightSensorFrontLeft | 0x011 | 3 | 0 | State | u8 enum | | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 | LE | TBD (raw, unit undecided) |
| HeightSensorFrontRight | 0x012 | 3 | 0 | State | u8 enum | | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 | LE | TBD (raw, unit undecided) |
| HeightSensor (placement TBD) | 0x013 | 3 | 0 | State | u8 enum | | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 | LE | TBD (raw, unit undecided) |
| HeightSensor (placement TBD) | 0x014 | 3 | 0 | State | u8 enum | | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 | LE | TBD (raw, unit undecided) |

## Bootloader

The bootloader occupies `0x030`–`0x03F`. Because several boards share the bus, each board type owns
its own block of three IDs derived from its application type, so a command can only ever reach the
board it is addressed to — the bootloader's hardware filter rejects the other blocks outright. This
is what keeps an `EraseApp` aimed at one board from erasing the others.

**Address formula:** `base = 0x031 + (app_type - 1) * 3`

| Board | App type | Command (host→board) | Response (board→host) | Write data (host→board) |
| --- | --- | --- | --- | --- |
| Rudder Controller | 0x01 | 0x031 | 0x032 | 0x033 |
| Height Sensor Controller | 0x02 | 0x034 | 0x035 | 0x036 |
| Dashboard | 0x03 | 0x037 | 0x038 | 0x039 |
| _(unallocated)_ | 0x04 | 0x03A | 0x03B | 0x03C |
| _(unallocated)_ | 0x05 | 0x03D | 0x03E | 0x03F |

`0x030` is a discovery broadcast used to enumerate the bus: the host sends `GetState` on it and every
bootloader answers on its **own** response ID. Replies are therefore attributed by source ID, and no
two nodes ever transmit the same identifier — a shared reply ID would have all boards win arbitration
together on the identical ID field and then collide in the data field. Answering discovery does not
extend a board's auto-boot window, so repeated scanning cannot hold boards in the bootloader.

A sixth application type would overflow the block and needs the allocation extended into `0x040`+.

In the table below, `cmd`, `resp` and `data` mean the addressed board's three IDs.

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| BootloaderDiscovery | 0x030 | 1 | 0 | Command | u8 enum | | 0x01=GetState. Broadcast; every bootloader answers on its own `resp` ID. |
| BootloaderHostToDevice | cmd | 1 | 0 | Command | u8 enum | | 0x01=GetState, 0x02=EraseApp, 0x04=ValidateApp, 0x05=BootApp, 0x06=Reboot |
| BootloaderDeviceToHost (GetState) | resp | 3 | 0 | Message type | u8 enum | | 0x01=GetState, 0x02=EraseApp, 0x03=WriteAck, 0x04=ValidateApp, 0x05=BootApp, 0x06=Reboot, 0xFF=Error |
| | | | 1 | State | u8 enum | | 0=WaitingWithoutApp, 1=WaitingWithApp, 2=FlashingApp |
| | | | 2 | App type | u8 enum | | 0x01=RudderController, 0x02=HeightSensorController, 0x03=Dashboard. The `resp` ID already implies this; the byte lets the host cross-check which board it reached. |
| BootloaderDeviceToHost (WriteAck) | resp | 5 | 0 | Message type | u8 | | 0x03 |
| | | | 1–4 | Write offset | u32 | LE | Cumulative bytes written |
| BootloaderDeviceToHost (ValidateApp) | resp | 2 | 0 | Message type | u8 | | 0x04 |
| | | | 1 | Result | u8 enum | | 0=Valid, 1=BadMagic, 2=BadLength, 3=BadCrc, 4=WrongAppType |
| BootloaderDeviceToHost (Error) | resp | 2 | 0 | Message type | u8 | | 0xFF |
| | | | 1 | Error code | u8 enum | | 0x01=EraseFailure, 0x02=InvalidWriteLength, 0x03=FlashWriteFailure, 0x04=InvalidApp, 0x05=UnknownCommand |
| BootloaderWriteData | data | 8 | 0–7 | Data | u8[8] | | Raw firmware data (8-byte aligned). No type byte, so writes stay a full 8 bytes per frame. |

A running application does not answer the bootloader protocol, so the host sends `Reboot` to the
board's `cmd` ID first. Applications use an accept-all filter, so they scope the reset by checking
the command ID against their own application type — rebooting one board leaves the others running.

> **Superseded allocation.** Before this scheme the bootloader used a flat `0x030` (host→device),
> `0x031` (device→host) and `0x032` (write data) shared by every board, which made `EraseApp` a
> broadcast. Note that `0x032` is now the Rudder Controller's **response** ID: a bootloader predating
> this change would read those responses as firmware write data. The bootloader is only replaceable
> over SWD, so every board must be re-flashed before the new host tooling is used on a shared bus.

## Battery Management System (BMS)

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| PackAndPerriCurrent | 0x100 | 8 | 0–3 | Pack current | f32 | LE | Amperes |
| | | | 4–7 | Perri current | f32 | LE | Amperes |
| ChargeAndDischargeCurrent | 0x101 | 8 | 0–3 | Charge current | f32 | LE | Amperes |
| | | | 4–7 | Discharge current | f32 | LE | Amperes (negated on wire) |
| SocErrorFlagsAndBalancing | 0x102 | 8 | 0–1 | State of charge | u16 | LE | raw / 100 = % |
| | | | 2–5 | Error flags | u32 | LE | Bitfield |
| | | | 6–7 | Balancing status | u16 | LE | Bitfield |
| CellVoltages1To4 | 0x103 | 8 | 0–1 | Cell 1 voltage | u16 | LE | raw / 1000 = V |
| | | | 2–3 | Cell 2 voltage | u16 | LE | raw / 1000 = V |
| | | | 4–5 | Cell 3 voltage | u16 | LE | raw / 1000 = V |
| | | | 6–7 | Cell 4 voltage | u16 | LE | raw / 1000 = V |
| CellVoltages5To8 | 0x104 | 8 | 0–1 | Cell 5 voltage | u16 | LE | raw / 1000 = V |
| | | | 2–3 | Cell 6 voltage | u16 | LE | raw / 1000 = V |
| | | | 4–5 | Cell 7 voltage | u16 | LE | raw / 1000 = V |
| | | | 6–7 | Cell 8 voltage | u16 | LE | raw / 1000 = V |
| CellVoltages9To12 | 0x105 | 8 | 0–1 | Cell 9 voltage | u16 | LE | raw / 1000 = V |
| | | | 2–3 | Cell 10 voltage | u16 | LE | raw / 1000 = V |
| | | | 4–5 | Cell 11 voltage | u16 | LE | raw / 1000 = V |
| | | | 6–7 | Cell 12 voltage | u16 | LE | raw / 1000 = V |
| CellVoltages13To14PackAndStack | 0x106 | 8 | 0–1 | Cell 13 voltage | u16 | LE | raw / 1000 = V |
| | | | 2–3 | Cell 14 voltage | u16 | LE | raw / 1000 = V |
| | | | 4–5 | Pack voltage | u16 | LE | raw / 1000 = V |
| | | | 6–7 | Stack voltage | u16 | LE | raw / 1000 = V |
| TemperaturesAndStates | 0x107 | 8 | 0 | Temperature 1 | i8 | | Celsius |
| | | | 1 | Temperature 2 | i8 | | Celsius |
| | | | 2 | Temperature 3 | i8 | | Celsius |
| | | | 3 | Temperature 4 | i8 | | Celsius |
| | | | 4 | IC temperature | i8 | | Celsius |
| | | | 5 | Battery state | u8 enum | | 0=Init, 1=Sleep, 2=WaitingForStartup, 3=Idle, 4=OnlyCharge, 5=OnlyDischarge, 6=On |
| | | | 6 | Charge state | u8 enum | | 0=Init, 1=Idle, 2=RelayOn, 3=FetOn, 4=Error, 5=FetOff |
| | | | 7 | Discharge state | u8 enum | | 0=Init, 1=Idle, 2=PreChargeOn, 3=On, 4=PreChargeTimeout, 5=Error |
| BatteryUptime | 0x108 | 4 | 0–3 | Uptime | u32 | LE | Milliseconds |

## GNSS

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| GnssStatus | 0x200 | 3 | 0 | Fix | u8 enum | | 0=No fix, 1=3D fix, 2=2D fix |
| | | | 1 | Satellites | u8 | | Count |
| | | | 2 | Satellites used | u8 | | Count |
| GnssSpeedAndHeading | 0x201 | 8 | 0–3 | Speed | f32 | LE | km/h |
| | | | 4–7 | Heading | f32 | LE | Degrees |
| GnssLatitude | 0x202 | 8 | 0–7 | Latitude | f64 | LE | Degrees |
| GnssLongitude | 0x203 | 8 | 0–7 | Longitude | f64 | LE | Degrees |
| GnssDateTime | 0x204 | 7 | 0–1 | Year | u16 | LE | e.g. 2024 |
| | | | 2 | Month | u8 | | 1–12 |
| | | | 3 | Day | u8 | | 1–31 |
| | | | 4 | Hours | u8 | | 0–23 |
| | | | 5 | Minutes | u8 | | 0–59 |
| | | | 6 | Seconds | u8 | | 0–59 |

## Data Logger

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| DataLoggerWifiIp | 0x205 | 4 | 0–3 | IPv4 address octets | u8 ×4 | | Address order: `192.168.1.5` → `C0 A8 01 05`. Sent every 1 s while the data logger has a WiFi IPv4; not sent otherwise |

## Controller Temperatures

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| TemperatureHeightSensorsController | 0x210 | 2 | 0–1 | Temperature | i16 | LE | Centidegrees Celsius |
| TemperatureRudderController | 0x211 | 2 | 0–1 | Temperature | i16 | LE | Centidegrees Celsius |

## Motor NTC Sensor

A standalone node with a 10 kΩ NTC on the motor, transmit only, never receives. It
supersedes reading the motor NTC through the VESC, whose own reading is broken, and
through the rudder controller's retired 0x217. It is what the display shows as
`Motor` and the only motor temperature on the bus.

Two firmwares produce this frame, identically, and **only one of them may be on a bus
at a time** — both transmit unprompted on `0x219`, which is a collision, not a
redundancy:

- `can-motor-temperature`, an STM32G491 on CANable 2.5 hardware. Period comes from
  its internal LSI, so 1 s ±5 %.
- [`motor-ntc-sensor`](firmware/README.md#motor-ntc-sensor), rudder controller
  hardware with the NTC on the steering potentiometer input. Period comes from the
  PLL, so a solid 1 s. It never sets bit 4 (AcquisitionError), and its bit 5
  (CanTxFailed) means the frame could not be queued rather than that it went
  unacknowledged.

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| MotorNtc | 0x219 | 4 | 0–1 | Temperature | i16 | LE | Decidegrees Celsius — note, not the centidegrees 0x210–0x217 use. `0x8000` (`-32768`) is the explicit invalid sentinel: no reading, see the status byte. Valid readings are clamped to -40.0…+150.0 °C. Sent every 1 s (±5 % on the CANable node, whose timebase is its internal LSI). |
| | | | 2 | Status | u8 | | Bit flags, see below. |
| | | | 3 | Frame counter | u8 | | Increments once per transmission and wraps. A gap means frames never reached the bus. The node can be built with DLC 2, which omits this byte and the status byte. |

Status bits in byte 2:

| Bit | Name | Meaning | Temperature |
| --- | --- | --- | --- |
| 0 | SensorOpen | NTC disconnected; the divider tap sits at the bias rail | sentinel |
| 1 | SensorShort | NTC shorted; the divider tap sits at ground | sentinel |
| 2 | OutOfRange | Reading fell outside -40…+150 °C | clamped, still usable |
| 3 | Settling | The node's filter has not seen enough updates; clears a few seconds after power-up | usable, still converging |
| 4 | AcquisitionError | The node's ADC or DMA delivered no samples this cycle | sentinel |
| 5 | PrevTxFailed | The *previous* frame was not acknowledged within 10 ms and was cancelled | unaffected |

Source and DBC: `boat-fw/can-motor-temperature` on git.engineersofinnovation.nl.

## VESC Motor Controller

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| VescStatusMessage1 | 0x0909 | 8 | 0–3 | RPM | i32 | BE | RPM |
| | | | 4–5 | Total current | i16 | BE | raw / 10 = A |
| | | | 6–7 | Duty cycle | i16 | BE | raw / 10 = % |
| VescStatusMessage2 | 0x0E09 | 8 | 0–3 | Amp hours used | u32 | BE | raw / 10000 = Ah |
| | | | 4–7 | Amp hours generated | u32 | BE | raw / 10000 = Ah |
| VescStatusMessage3 | 0x0F09 | 8 | 0–3 | Watt hours used | u32 | BE | raw / 10000 = Wh |
| | | | 4–7 | Watt hours generated | u32 | BE | raw / 10000 = Wh |
| VescStatusMessage4 | 0x1009 | 8 | 0–1 | FET temperature | i16 | BE | raw / 10 = °C |
| | | | 2–3 | Motor temperature | i16 | BE | raw / 10 = °C. **Broken on this boat** — the motor temperature comes from `MotorNtc` (0x219) instead. |
| | | | 4–5 | Total input current | i16 | BE | raw / 10 = A |
| | | | 6–7 | Current PID position | i16 | BE | raw / 50 |
| VescStatusMessage5 | 0x1B09 | 8 | 0–3 | Tachometer | i32 | BE | Counts |
| | | | 4–5 | Input voltage | i16 | BE | raw / 10 = V |

## Throttle Controller

| Message | CAN ID | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ThrottleToVescDutyCycle | 0x0009 | 4 | 0–3 | Duty cycle | i32 | BE | raw / 1000 = % |
| ThrottleToVescCurrent | 0x0109 | 4 | 0–3 | Current | i32 | BE | raw / 1000 = A |
| ThrottleToVescCurrentRelative | 0x0A09 | 4 | 0–3 | Relative current | i32 | BE | raw / 1000 = % |
| ThrottleToVescRpm | 0x0309 | 4 | 0–3 | RPM | i32 | BE | raw / 1000 = RPM |
| ThrottleStatus | 0x1337 or 0x0337 (DLC=8) | 8 | 0–1 | Throttle value | i16 | BE | (raw / 512) × 100 = % |
| | | | 2–3 | Raw angle | i16 | BE | Counts |
| | | | 4–5 | Raw deadman | i16 | BE | Counts |
| | | | 6 | Gain | u8 | | 0–255 |
| | | | 7 | Error flags | u8 bitfield | | bits 0–2=TWI error state, bit 3=NoEeprom, bit 4=GainClipping, bit 5=GainInvalid, bit 6=DeadmanMissing, bit 7=ImpedanceHigh |
| ThrottleConfig | 0x1337 or 0x0337 (DLC=6) | 6 | 0 | Marker | u8 | | Must be 0xAA; otherwise frame is ignored |
| | | | 1 | Control type | u8 enum | | 1=FilteredDutyCycle, 2=DutyCycle, 3=Current, 4=Rpm, 5=CurrentRelative |
| | | | 2–3 | Lever forward | i16 | BE | Counts |
| | | | 4–5 | Lever backward | i16 | BE | Counts |

## MPPT Solar Controllers

MPPT controllers occupy IDs `0x700`–`0x77F` (up to 8 devices, 16 info fields each).

Address formula: `CAN ID = 0x700 | (mppt_id << 4) | field_id`

- `mppt_id` = bits 6–4 of the lower byte (0–7, selects the device)
- `field_id` = bits 3–0 of the lower byte (0–15, selects the info type)

| Message | field_id | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| MpptChannelPower (ch 0) | 0 | 8 | 0–3 | Voltage in | f32 | LE | V |
| | | | 4–7 | Current in | f32 | LE | A |
| MpptChannelState (ch 0) | 1 | 5 | 0–1 | Duty cycle | u16 | LE | |
| | | | 2 | Algorithm | u8 | | |
| | | | 3 | Algorithm state | u8 | | |
| | | | 4 | Channel active | u8 bool | | 0=Inactive, 1=Active |
| MpptChannelPower (ch 1) | 2 | 8 | 0–3 | Voltage in | f32 | LE | V |
| | | | 4–7 | Current in | f32 | LE | A |
| MpptChannelState (ch 1) | 3 | 5 | 0–1 | Duty cycle | u16 | LE | |
| | | | 2 | Algorithm | u8 | | |
| | | | 3 | Algorithm state | u8 | | |
| | | | 4 | Channel active | u8 bool | | 0=Inactive, 1=Active |
| MpptChannelPower (ch 2) | 4 | 8 | 0–3 | Voltage in | f32 | LE | V |
| | | | 4–7 | Current in | f32 | LE | A |
| MpptChannelState (ch 2) | 5 | 5 | 0–1 | Duty cycle | u16 | LE | |
| | | | 2 | Algorithm | u8 | | |
| | | | 3 | Algorithm state | u8 | | |
| | | | 4 | Channel active | u8 bool | | 0=Inactive, 1=Active |
| MpptChannelPower (ch 3) | 6 | 8 | 0–3 | Voltage in | f32 | LE | V |
| | | | 4–7 | Current in | f32 | LE | A |
| MpptChannelState (ch 3) | 7 | 5 | 0–1 | Duty cycle | u16 | LE | |
| | | | 2 | Algorithm | u8 | | |
| | | | 3 | Algorithm state | u8 | | |
| | | | 4 | Channel active | u8 bool | | 0=Inactive, 1=Active |
| MpptPower | 8 | 8 | 0–3 | Voltage out | f32 | LE | V |
| | | | 4–7 | Current out | f32 | LE | A |
| MpptStatus | 9 | 8 | 0–3 | Voltage out (switch) | f32 | LE | V |
| | | | 4–5 | Temperature | i16 | LE | °C |
| | | | 6 | State | u8 | | |
| | | | 7 | Flags | u8 bitfield | | bit 0=PWM enabled, bit 1=switch on |

## GaN MPPT Solar Controllers

GaN MPPT controllers occupy IDs `0x400`–`0x4FF` (up to 16 nodes).

Address formula: `CAN ID = ((node_id + 64) << 4) | packet_id`

- `node_id` = 0–15 (hardware-offset node index)
- `packet_id` = bits 3–0 (0–2, selects the packet type)

| Message | packet_id | DLC | Byte | Field | Type | Endian | Values / Range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| GanMpptPower | 0x00 | 8 | 0–1 | Input voltage | i16 | BE | raw / 100 = V |
| | | | 2–3 | Input current | i16 | BE | raw / 2000 = A |
| | | | 4–5 | Output voltage | i16 | BE | raw / 100 = V |
| | | | 6–7 | Output current | i16 | BE | raw / 2000 = A |
| GanMpptStatus | 0x01 | 5 | 0 | Mode | u8 enum | | 0=None, 1=Civ, 2=Cic, 3=MinInputCurrent, 4=Cov, 5=Coc, 6=TemperatureDerating, 7=Fault |
| | | | 1 | Fault | u8 enum | | 0=Ok, 1=ConfigError, 2=InputOverVoltage, 3=OutputOverVoltage, 4=OutputOverCurrent, 5=InputOverCurrent, 6=InputUnderCurrent, 7=PhaseOverCurrent, 8=GeneralFault |
| | | | 2 | Enabled | u8 bool | | 0=Disabled, 1=Enabled |
| | | | 3 | Board temperature | i8 | | °C |
| | | | 4 | Heat sink temperature | i8 | | °C |
| GanMpptSweepData | 0x02 | 5 | 0 | Index | u8 | | Sweep point index |
| | | | 1–2 | Current | i16 | BE | raw / 2000 = A |
| | | | 3–4 | Voltage | i16 | BE | raw / 100 = V |
