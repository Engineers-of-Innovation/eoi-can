//! The Linux framebuffer (`/dev/fb0` on the datalogger's DSI screen).
//!
//! Pixels are packed from the bitfields the kernel reports, so RGB565, RGB888 and
//! XRGB8888 all work, and rows are written `line_length` apart, which can be wider
//! than the visible width.

use framebuffer::{Bitfield, Framebuffer};
use tiny_skia::Pixmap;

pub struct Fb {
    fb: Framebuffer,
    bytes_pp: usize,
    line: Vec<u8>,
}

fn pack(channel: u8, field: &Bitfield) -> u32 {
    if field.length == 0 {
        return 0;
    }
    (u32::from(channel) >> (8 - field.length.min(8))) << field.offset
}

impl Fb {
    pub fn open(device: &str) -> Result<Self, String> {
        let fb = Framebuffer::new(device).map_err(|e| format!("{device}: {}", e.details))?;
        let bpp = fb.var_screen_info.bits_per_pixel;
        if ![16, 24, 32].contains(&bpp) {
            return Err(format!("{device}: {bpp} bits per pixel is not supported"));
        }
        let v = &fb.var_screen_info;
        tracing::info!(
            "{device}: {}x{} at {bpp} bpp, line {} bytes, rgb offsets {}/{}/{}",
            v.xres,
            v.yres,
            fb.fix_screen_info.line_length,
            v.red.offset,
            v.green.offset,
            v.blue.offset
        );
        Ok(Self {
            bytes_pp: bpp as usize / 8,
            line: Vec::new(),
            fb,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.fb.var_screen_info.xres, self.fb.var_screen_info.yres)
    }

    /// Copies `pm` to the top left of the screen (it is normally exactly screen-sized).
    pub fn show(&mut self, pm: &Pixmap) {
        let v = self.fb.var_screen_info.clone();
        let stride = self.fb.fix_screen_info.line_length as usize;
        let origin = v.yoffset as usize * stride + v.xoffset as usize * self.bytes_pp;
        let (w, h) = (
            pm.width().min(v.xres) as usize,
            pm.height().min(v.yres) as usize,
        );
        let (red, green, blue) = (v.red, v.green, v.blue);
        let src_stride = pm.width() as usize * 4;
        self.line.resize(w * self.bytes_pp, 0);
        for y in 0..h {
            let src = &pm.data()[y * src_stride..][..w * 4];
            // Every pixel is opaque (the frame starts from a solid fill), so the
            // premultiplied RGBA is plain RGB.
            for (px, out) in src
                .as_chunks::<4>()
                .0
                .iter()
                .zip(self.line.chunks_exact_mut(self.bytes_pp))
            {
                let value = pack(px[0], &red) | pack(px[1], &green) | pack(px[2], &blue);
                out.copy_from_slice(&value.to_le_bytes()[..self.bytes_pp]);
            }
            let start = origin + y * stride;
            if let Some(dst) = self.fb.frame.get_mut(start..start + self.line.len()) {
                dst.copy_from_slice(&self.line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(offset: u32, length: u32) -> Bitfield {
        Bitfield {
            offset,
            length,
            msb_right: 0,
        }
    }

    #[test]
    fn rgb565() {
        let v = pack(0xFF, &field(11, 5)) | pack(0x80, &field(5, 6)) | pack(0x08, &field(0, 5));
        assert_eq!(v, 0b1111_1100_0000_0001);
    }

    #[test]
    fn xrgb8888() {
        let v = pack(0x12, &field(16, 8)) | pack(0x34, &field(8, 8)) | pack(0x56, &field(0, 8));
        assert_eq!(v.to_le_bytes(), [0x56, 0x34, 0x12, 0x00]);
    }
}
