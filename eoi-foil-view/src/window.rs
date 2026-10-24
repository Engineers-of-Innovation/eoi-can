//! An SDL window, for working on the display on a desktop (`--features window`).

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use tiny_skia::Pixmap;

/// Shows frames from `render` until the window is closed or Escape is pressed.
/// `render` is called about every `period`.
pub fn run(
    width: u32,
    height: u32,
    period: std::time::Duration,
    mut render: impl FnMut(&mut Pixmap),
) -> Result<(), String> {
    let sdl = sdl2::init()?;
    let window = sdl
        .video()?
        .window("EoI foil view", width, height)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    let creator = canvas.texture_creator();
    // ABGR8888 is R, G, B, A in memory on a little-endian host: the pixmap's order.
    let mut texture = creator
        .create_texture_streaming(PixelFormatEnum::ABGR8888, width, height)
        .map_err(|e| e.to_string())?;
    let mut events = sdl.event_pump()?;
    let mut pm = Pixmap::new(width, height).ok_or("bad window size")?;
    loop {
        let started = std::time::Instant::now();
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::ESCAPE),
                    ..
                } => return Ok(()),
                _ => {}
            }
        }
        render(&mut pm);
        texture
            .update(None, pm.data(), width as usize * 4)
            .map_err(|e| e.to_string())?;
        canvas.copy(&texture, None, None)?;
        canvas.present();
        if let Some(rest) = period.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}
