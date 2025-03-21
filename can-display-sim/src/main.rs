use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use embedded_graphics_simulator::{OutputSettingsBuilder, SimulatorDisplay, Window};

#[tokio::main]
async fn main() -> Result<(), core::convert::Infallible> {
    let mut display: SimulatorDisplay<BinaryColor> = SimulatorDisplay::new(Size::new(800, 480));

    draw_display::draw_display(&mut display)?;

    let output_settings = OutputSettingsBuilder::new().scale(1).max_fps(1).build();
    let mut window = Window::new(
        "Engineers of Innovation CAN Display Simulator",
        &output_settings,
    );

    window.show_static(&display);
    // window.update(&display);
    // sleep(Duration::from_secs(1)).await;

    Ok(())
}
