#![cfg_attr(not(test), no_std)]

mod time;

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_4X6},
        MonoTextStyle, MonoTextStyleBuilder,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle},
    text::{Alignment, Text},
};
use heapless::String;
use time::{Duration, Instant};

const DISPLAY_VALUE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DisplayValue<T> {
    value: Option<T>,
    last_updated: Instant,
}

impl<T> DisplayValue<T> {
    pub fn update(&mut self, value: T) {
        self.value = Some(value);
        self.last_updated = Instant::now();
    }

    pub fn is_valid(&self) -> bool {
        self.value.is_some() && self.last_updated.elapsed() < DISPLAY_VALUE_TIMEOUT
    }

    pub fn get(&self) -> Option<&T> {
        if self.is_valid() {
            self.value.as_ref()
        } else {
            None
        }
    }
}

impl<T> Default for DisplayValue<T> {
    fn default() -> Self {
        Self {
            value: None,
            last_updated: Instant::now(), // We need to set something as initial value, will be updated when first value is set
        }
    }
}

#[derive(Debug, Default)]
pub struct DisplayData {
    pub speed_kmh: DisplayValue<f32>,
    pub cell_voltage: [DisplayValue<f32>; 14],
}

pub fn draw_display<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let mut string_helper: String<64> = String::new();
    display.clear(BinaryColor::On.into())?;

    let zwarte_doos: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On.into())
        .background_color(BinaryColor::Off.into())
        .build();

    let normal: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();

    // power meter

    Line::new(Point::new(0, 70), Point::new(800, 70))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::Off.into(), 2))
        .draw(display)?;

    Text::with_alignment(
        "Charging disabled !!!",
        Point::new(400, 50),
        zwarte_doos,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment(
        "Net Power (W)",
        Point::new(300, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment("1500", Point::new(300, 130), normal, Alignment::Center).draw(display)?;

    Line::new(Point::new(0, 140), Point::new(800, 140))
        .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 2))
        .draw(display)?;

    Text::with_alignment(
        "Speed (km/h)",
        Point::new(100, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    if let Some(data) = data.speed_kmh.get() {
        write!(&mut string_helper, "{:.1}", data).unwrap();
    } else {
        string_helper.push_str("N/A").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(100, 130),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    // state of charge

    Text::with_alignment(
        "State of Charge (%)",
        Point::new(500, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment("66.6", Point::new(500, 130), normal, Alignment::Center).draw(display)?;

    Text::with_alignment(
        "Time to empty (Min)",
        Point::new(700, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment("80", Point::new(700, 130), normal, Alignment::Center).draw(display)?;

    // MPPT information

    Text::new(
        "MPPT",
        Point::new(15, 370),
        MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
    )
    .draw(display)?;
    let mut panel_text: String<64> = String::new();
    use core::fmt::Write;
    for panel in 0..11 {
        panel_text.clear();
        write!(&mut panel_text, "Panel {:2}   50 W 24 V 2 A", panel + 1).unwrap();
        Text::new(
            panel_text.as_str(),
            Point::new(15, (panel * 8) + 390),
            MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
        )
        .draw(display)?;
    }

    // battery information

    let mut battery_offset_y = 200;
    const FONT_10X20_SPACE: i32 = 20;

    Text::new("Battery", Point::new(415, 170), normal).draw(display)?;

    Text::new("Input 500 W", Point::new(415, battery_offset_y), normal).draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Output motor 1990 W",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Output peripherals 10 W",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Max temperature 25 C",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Min temperature 20 C",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Avg temperature 22 C",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Max cell voltage 4.123 V",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Min cell voltage 3.123 V",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Avg cell voltage 3.612 V",
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    let mut cell_text: String<64> = String::new();

    for cell in 0..7 {
        cell_text.clear();
        if let Some(cell_voltage) = data.cell_voltage[cell as usize].get() {
            write!(
                &mut cell_text,
                "Cell {:2}: {:1.3} V",
                cell + 1,
                cell_voltage
            )
            .unwrap();
        } else {
            write!(&mut cell_text, "Cell {:2}: N/A", cell + 1).unwrap();
        }

        Text::new(
            cell_text.as_str(),
            Point::new(650, (cell * 8) + 420),
            MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
        )
        .draw(display)?;
    }

    for cell in 7..14 {
        cell_text.clear();
        if let Some(cell_voltage) = data.cell_voltage[cell as usize].get() {
            write!(
                &mut cell_text,
                "Cell {:2}: {:1.3} V",
                cell + 1,
                cell_voltage
            )
            .unwrap();
        } else {
            write!(&mut cell_text, "Cell {:2}: N/A", cell + 1).unwrap();
        }
        Text::new(
            cell_text.as_str(),
            Point::new(730, ((cell - 7) * 8) + 420),
            MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
        )
        .draw(display)?;
    }

    Line::new(Point::new(400, 140), Point::new(400, 480))
        .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 2))
        .draw(display)?;

    // Create a new window
    let mut motordriver_offset_y = 200;

    Text::new("Motor driver", Point::new(15, 170), normal).draw(display)?;

    Text::new(
        "Battery power usage 1990 W",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Battery Current 30 A",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Motor Current 100 A",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Duty cycle 66.6%",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "FET temperature 25 C",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Motor temperature 20 C",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Throttle is ARMED",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    Text::new(
        "Throttle value -50%",
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    Text::new(
        "Last updated: 12:34:56",
        Point::new(300, 470),
        MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
    )
    .draw(display)?;

    Text::new(
        "Ip address: 127.0.0.1",
        Point::new(415, 470),
        MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
    )
    .draw(display)?;

    Ok(())
}
