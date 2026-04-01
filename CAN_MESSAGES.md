# CAN Messages

All multi-byte values are little-endian unless noted otherwise. Any state byte value not listed maps to `Unknown` on the receiver side.

## Rudder Controller

| Message | CAN ID | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| ServoRudderSetpoint | 0x010 | 2 | 0–1 | Setpoint | u16 LE | 1000–2000 |
| ServoRudderStatus | 0x020 | 3 | 0 | State | u8 enum | 0=Uninitialized, 1=Operational, 0xFF=Unknown |
| | | | 1–2 | Current setpoint | u16 LE | 1000–2000 |
| ServoRudderCommand | 0x021 | 1 | 0 | Command | u8 enum | 0=Initialize |

## Height Sensors

| Message | CAN ID | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| HeightSensorFrontLeft | 0x011 | 3 | 0 | State | u8 enum | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 LE | TBD (raw, unit undecided) |
| HeightSensorFrontRight | 0x012 | 3 | 0 | State | u8 enum | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 LE | TBD (raw, unit undecided) |
| HeightSensor (placement TBD) | 0x013 | 3 | 0 | State | u8 enum | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 LE | TBD (raw, unit undecided) |
| HeightSensor (placement TBD) | 0x014 | 3 | 0 | State | u8 enum | 0=NotPluggedIn, 1=ModbusError, 2=Operational, 0xFF=Unknown |
| | | | 1–2 | Height value | u16 LE | TBD (raw, unit undecided) |

## Battery Management System (BMS)

| Message | CAN ID | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| PackAndPerriCurrent | 0x100 | 8 | 0–3 | Pack current | f32 LE | Amperes |
| | | | 4–7 | Perri current | f32 LE | Amperes |
| ChargeAndDischargeCurrent | 0x101 | 8 | 0–3 | Charge current | f32 LE | Amperes |
| | | | 4–7 | Discharge current | f32 LE | Amperes (negated on wire) |
| SocErrorFlagsAndBalancing | 0x102 | 8 | 0–1 | State of charge | u16 LE | raw / 100 = % |
| | | | 2–5 | Error flags | u32 LE | Bitfield |
| | | | 6–7 | Balancing status | u16 LE | Bitfield |
| CellVoltages1To4 | 0x103 | 8 | 0–1 | Cell 1 voltage | u16 LE | raw / 1000 = V |
| | | | 2–3 | Cell 2 voltage | u16 LE | raw / 1000 = V |
| | | | 4–5 | Cell 3 voltage | u16 LE | raw / 1000 = V |
| | | | 6–7 | Cell 4 voltage | u16 LE | raw / 1000 = V |
| CellVoltages5To8 | 0x104 | 8 | 0–1 | Cell 5 voltage | u16 LE | raw / 1000 = V |
| | | | 2–3 | Cell 6 voltage | u16 LE | raw / 1000 = V |
| | | | 4–5 | Cell 7 voltage | u16 LE | raw / 1000 = V |
| | | | 6–7 | Cell 8 voltage | u16 LE | raw / 1000 = V |
| CellVoltages9To12 | 0x105 | 8 | 0–1 | Cell 9 voltage | u16 LE | raw / 1000 = V |
| | | | 2–3 | Cell 10 voltage | u16 LE | raw / 1000 = V |
| | | | 4–5 | Cell 11 voltage | u16 LE | raw / 1000 = V |
| | | | 6–7 | Cell 12 voltage | u16 LE | raw / 1000 = V |
| CellVoltages13To14PackAndStack | 0x106 | 8 | 0–1 | Cell 13 voltage | u16 LE | raw / 1000 = V |
| | | | 2–3 | Cell 14 voltage | u16 LE | raw / 1000 = V |
| | | | 4–5 | Pack voltage | u16 LE | raw / 1000 = V |
| | | | 6–7 | Stack voltage | u16 LE | raw / 1000 = V |
| TemperaturesAndStates | 0x107 | 8 | 0 | Temperature 1 | i8 | Celsius |
| | | | 1 | Temperature 2 | i8 | Celsius |
| | | | 2 | Temperature 3 | i8 | Celsius |
| | | | 3 | Temperature 4 | i8 | Celsius |
| | | | 4 | IC temperature | i8 | Celsius |
| | | | 5 | Battery state | u8 enum | 0=Init, 1=Sleep, 2=WaitingForStartup, 3=Idle, 4=OnlyCharge, 5=OnlyDischarge, 6=On |
| | | | 6 | Charge state | u8 enum | 0=Init, 1=Idle, 2=RelayOn, 3=FetOn, 4=Error, 5=FetOff |
| | | | 7 | Discharge state | u8 enum | 0=Init, 1=Idle, 2=PreChargeOn, 3=On, 4=PreChargeTimeout, 5=Error |
| BatteryUptime | 0x108 | 4 | 0–3 | Uptime | u32 LE | Milliseconds |

## GNSS

| Message | CAN ID | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| GnssStatus | 0x200 | 3 | 0 | Fix | u8 | 0=No fix, 1=3D fix |
| | | | 1 | Satellites | u8 | Count |
| | | | 2 | Satellites used | u8 | Count |
| GnssSpeedAndHeading | 0x201 | 8 | 0–3 | Speed | f32 LE | km/h |
| | | | 4–7 | Heading | f32 LE | Degrees |
| GnssLatitude | 0x202 | 8 | 0–7 | Latitude | f64 LE | Degrees |
| GnssLongitude | 0x203 | 8 | 0–7 | Longitude | f64 LE | Degrees |
| GnssDateTime | 0x204 | 7 | 0–1 | Year | u16 LE | e.g. 2024 |
| | | | 2 | Month | u8 | 1–12 |
| | | | 3 | Day | u8 | 1–31 |
| | | | 4 | Hours | u8 | 0–23 |
| | | | 5 | Minutes | u8 | 0–59 |
| | | | 6 | Seconds | u8 | 0–59 |

## VESC Motor Controller

> All VESC messages use **big-endian** byte order.

| Message | CAN ID | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| VescStatusMessage1 | 0x0909 | 8 | 0–3 | RPM | i32 BE | RPM |
| | | | 4–5 | Total current | i16 BE | raw / 10 = A |
| | | | 6–7 | Duty cycle | i16 BE | raw / 10 = % |
| VescStatusMessage2 | 0x0E09 | 8 | 0–3 | Amp hours used | u32 BE | raw / 10000 = Ah |
| | | | 4–7 | Amp hours generated | u32 BE | raw / 10000 = Ah |
| VescStatusMessage3 | 0x0F09 | 8 | 0–3 | Watt hours used | u32 BE | raw / 10000 = Wh |
| | | | 4–7 | Watt hours generated | u32 BE | raw / 10000 = Wh |
| VescStatusMessage4 | 0x1009 | 8 | 0–1 | FET temperature | i16 BE | raw / 10 = °C |
| | | | 2–3 | Motor temperature | i16 BE | raw / 10 = °C |
| | | | 4–5 | Total input current | i16 BE | raw / 10 = A |
| | | | 6–7 | Current PID position | i16 BE | raw / 50 |
| VescStatusMessage5 | 0x1B09 | 8 | 0–3 | Tachometer | i32 BE | Counts |
| | | | 4–5 | Input voltage | i16 BE | raw / 10 = V |

## Throttle Controller

> All throttle messages use **big-endian** byte order.

| Message | CAN ID | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| ThrottleToVescDutyCycle | 0x0009 | 4 | 0–3 | Duty cycle | i32 BE | raw / 1000 = % |
| ThrottleToVescCurrent | 0x0109 | 4 | 0–3 | Current | i32 BE | raw / 1000 = A |
| ThrottleToVescRpm | 0x0309 | 4 | 0–3 | RPM | i32 BE | raw / 1000 = RPM |
| ThrottleStatus | 0x1337 or 0x0337 (DLC=8) | 8 | 0–1 | Throttle value | i16 BE | (raw / 512) × 100 = % |
| | | | 2–3 | Raw angle | i16 BE | Counts |
| | | | 4–5 | Raw deadman | i16 BE | Counts |
| | | | 6 | Gain | u8 | 0–255 |
| | | | 7 | Error flags | u8 bitfield | bits 0–2=TWI error state, bit 3=NoEeprom, bit 4=GainClipping, bit 5=GainInvalid, bit 6=DeadmanMissing, bit 7=ImpedanceHigh |
| ThrottleConfig | 0x1337 or 0x0337 (DLC=6) | 6 | 0 | Control type | u8 enum | 0=DutyCycle, 1=FilteredDutyCycle, 2=Current, 3=Rpm, 4=CurrentRelative |
| | | | 1–2 | Lever forward | i16 BE | Counts |
| | | | 3–4 | Lever backward | i16 BE | Counts |

## MPPT Solar Controllers

MPPT controllers occupy IDs `0x700`–`0x77F` (up to 8 devices, 16 info fields each).

Address formula: `CAN ID = 0x700 | (mppt_id << 4) | field_id`

- `mppt_id` = bits 6–4 of the lower byte (0–7, selects the device)
- `field_id` = bits 3–0 of the lower byte (0–15, selects the info type)

| Message | field_id | DLC | Byte | Field | Type | Values / Range |
|---|---|---|---|---|---|---|
| MpptChannelPower (ch 0) | 0 | 8 | 0–3 | Voltage in | f32 LE | V |
| | | | 4–7 | Current in | f32 LE | A |
| MpptChannelState (ch 0) | 1 | 5 | 0–1 | Duty cycle | u16 LE | |
| | | | 2 | Algorithm | u8 | |
| | | | 3 | Algorithm state | u8 | |
| | | | 4 | Channel active | u8 bool | 0=Inactive, 1=Active |
| MpptChannelPower (ch 1) | 2 | 8 | 0–3 | Voltage in | f32 LE | V |
| | | | 4–7 | Current in | f32 LE | A |
| MpptChannelState (ch 1) | 3 | 5 | 0–1 | Duty cycle | u16 LE | |
| | | | 2 | Algorithm | u8 | |
| | | | 3 | Algorithm state | u8 | |
| | | | 4 | Channel active | u8 bool | 0=Inactive, 1=Active |
| MpptChannelPower (ch 2) | 4 | 8 | 0–3 | Voltage in | f32 LE | V |
| | | | 4–7 | Current in | f32 LE | A |
| MpptChannelState (ch 2) | 5 | 5 | 0–1 | Duty cycle | u16 LE | |
| | | | 2 | Algorithm | u8 | |
| | | | 3 | Algorithm state | u8 | |
| | | | 4 | Channel active | u8 bool | 0=Inactive, 1=Active |
| MpptChannelPower (ch 3) | 6 | 8 | 0–3 | Voltage in | f32 LE | V |
| | | | 4–7 | Current in | f32 LE | A |
| MpptChannelState (ch 3) | 7 | 5 | 0–1 | Duty cycle | u16 LE | |
| | | | 2 | Algorithm | u8 | |
| | | | 3 | Algorithm state | u8 | |
| | | | 4 | Channel active | u8 bool | 0=Inactive, 1=Active |
| MpptPower | 8 | 8 | 0–3 | Voltage out | f32 LE | V |
| | | | 4–7 | Current out | f32 LE | A |
| MpptStatus | 9 | 8 | 0–3 | Voltage out (switch) | f32 LE | V |
| | | | 4–5 | Temperature | i16 LE | °C |
| | | | 6 | State | u8 | |
| | | | 7 | Flags | u8 bitfield | bit 0=PWM enabled, bit 1=switch on |
