//! Mode 7: the rotation and scaling plane, with the matrix the PPU latched for the line.

use crate::{Pixel, Renderer, WIDTH};
use ssd_oracle::LineRegs;

/// The Mode 7 plane is 128 x 128 tiles.
const PLANE_PIXELS: i64 = 128 * 8;

/// The lowest 10 bits as the magnitude and the sign of the whole value (not a clip to 11 bits),
/// as the PPU does.
fn clip_i11(value: i32) -> i32 {
    let magnitude = value & 0x03FF;
    let sign = (value >> 31) & !0x03FF;
    magnitude | sign
}

fn sign_extend_i13(value: u16) -> i32 {
    (((value as i16) << 3) >> 3).into()
}

/// Intermediate results lose their lowest 6 bits.
fn truncate(value: i64) -> i64 {
    value & !0x3F
}

/// A screen coordinate of the line, `sub` of `scale` steps into the pixel whose (already flipped)
/// position is `coord`, in units of 1/`scale` of a pixel; flipped, the count runs from the right
/// (or the bottom) of the screen.
fn units(coord: i64, sub: i64, flip: bool, scale: i64) -> i64 {
    if flip { coord * scale - sub } else { coord * scale + sub }
}

/// The plane of one line: its matrix, centre and scroll, as the PPU latched them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plane {
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    center_x: i64,
    center_y: i64,
    /// The scroll less the centre, clipped to 11 bits as the PPU clips it.
    scrolled_x: i64,
    scrolled_y: i64,
    out_of_bounds: u8,
    h_flip: bool,
    v_flip: bool,
}

impl Plane {
    /// The plane of the line `regs` describes.
    pub(crate) fn of(regs: &LineRegs) -> Self {
        let center_x = sign_extend_i13(regs.m7_center_x);
        let center_y = sign_extend_i13(regs.m7_center_y);
        let h_scroll = sign_extend_i13(regs.m7_h_scroll);
        let v_scroll = sign_extend_i13(regs.m7_v_scroll);
        Self {
            a: i64::from(regs.m7_a as i16),
            b: i64::from(regs.m7_b as i16),
            c: i64::from(regs.m7_c as i16),
            d: i64::from(regs.m7_d as i16),
            center_x: i64::from(center_x),
            center_y: i64::from(center_y),
            scrolled_x: i64::from(clip_i11(h_scroll - center_x)),
            scrolled_y: i64::from(clip_i11(v_scroll - center_y)),
            out_of_bounds: regs.m7_out_of_bounds,
            h_flip: regs.m7_h_flip,
            v_flip: regs.m7_v_flip,
        }
    }

    /// The colour of the plane (0 when transparent) at the screen position `x` and `y` of the line:
    /// each a pixel and a sub-position `sub` of `0..scale` inside it. `x` is a window position: it
    /// lies outside `0..256` in a wide view, where the plane continues past the screen's edges. At `scale` 1 that is the
    /// PPU's own sample; at a larger scale the matrix is applied to the finer position.
    pub(crate) fn sample(&self, vram: &[u16], scale: i64, x: (i32, i64), y: (u16, i64)) -> u8 {
        let (map_x, map_y) = self.map_position(scale, x, y);
        self.fetch(vram, map_x >> 8, map_y >> 8).0
    }

    /// The place of the plane the screen position (as in [`Plane::sample`]) shows, in 1/256 of a
    /// plane pixel, before it is wrapped or clipped to the plane.
    pub(crate) fn map_position(&self, scale: i64, x: (i32, i64), y: (u16, i64)) -> (i64, i64) {
        let x_pixel = if self.h_flip { 255 - i64::from(x.0) } else { i64::from(x.0) };
        let y_pixel =
            if self.v_flip { i64::from(255_u16.wrapping_sub(y.0)) } else { i64::from(y.0) };
        let screen_x = units(x_pixel, x.1, self.h_flip, scale);
        let screen_y = units(y_pixel, y.1, self.v_flip, scale);
        let map_x = truncate(self.a * self.scrolled_x)
            + (self.a * screen_x).div_euclid(scale)
            + truncate(self.b * self.scrolled_y)
            + truncate((self.b * screen_y).div_euclid(scale))
            + (self.center_x << 8);
        let map_y = truncate(self.c * self.scrolled_x)
            + (self.c * screen_x).div_euclid(scale)
            + truncate(self.d * self.scrolled_y)
            + truncate((self.d * screen_y).div_euclid(scale))
            + (self.center_y << 8);
        (map_x, map_y)
    }

    /// The colour index of the plane at the plane pixel `(map_x, map_y)` (outside the plane as the
    /// PPU's out-of-bounds setting says), and whether the pixel is a place of the plane itself (inside
    /// it, or wrapped round it: the art's), not the transparent or the character-0 filler.
    pub(crate) fn fetch(&self, vram: &[u16], mut map_x: i64, mut map_y: i64) -> (u8, bool) {
        let mut tile_zero = false;
        if !(0..PLANE_PIXELS).contains(&map_x) || !(0..PLANE_PIXELS).contains(&map_y) {
            match self.out_of_bounds {
                0 => {
                    map_x &= PLANE_PIXELS - 1;
                    map_y &= PLANE_PIXELS - 1;
                }
                1 => return (0, false),
                _ => {
                    map_x &= 7;
                    map_y &= 7;
                    tile_zero = true;
                }
            }
        }

        let tile = if tile_zero {
            0
        } else {
            let address = (map_y / 8) * (PLANE_PIXELS / 8) + map_x / 8;
            usize::from(vram[address as usize] & 0x00FF)
        };
        let address = 64 * tile + 8 * (map_y % 8) as usize + (map_x % 8) as usize;
        ((vram[address & 0x7FFF] >> 8) as u8, !tile_zero)
    }

    /// How many art pixels of an art that has `art_scale` of them to a game pixel one picture
    /// pixel of `scale` to a game pixel spans on this line, in 1/256: the longer of its steps
    /// along a row and down a column.
    pub(crate) fn footprint(&self, scale: i64, art_scale: i64) -> i64 {
        let length = |p: i64, q: i64| (p * p + q * q).isqrt();
        length(self.a, self.c).max(length(self.b, self.d)) * art_scale / scale
    }
}

impl Renderer {
    pub(crate) fn render_mode_7(
        &mut self,
        vram: &[u16],
        regs: &LineRegs,
        scanline: u16,
        from: usize,
    ) {
        let plane = Plane::of(regs);
        // The mosaic of a wide view is laid by its chunks (the plane is drawn unmosaicked there), so
        // the blocks here are the screen's own, from x = 0.
        let mosaic = regs.mosaic_bg[0];
        debug_assert!(self.origin == 0 || !mosaic, "a wide chunk lays its mosaic itself");
        let block = u16::from(regs.mosaic_size) + 1;
        let base_y =
            if mosaic { scanline.wrapping_sub(u16::from(regs.mosaic_v_offset)) } else { scanline };

        for pixel in from..WIDTH {
            let pixel = pixel as u16;
            if mosaic && pixel / block * block != pixel {
                self.bg_pixels[0][usize::from(pixel)] = self.bg_pixels[0][usize::from(pixel - 1)];
                continue;
            }
            // The plane continues past the screen's edges in a wide view: x is the window's
            let color = plane.sample(vram, 1, (self.origin + i32::from(pixel), 0), (base_y, 0));
            self.bg_pixels[0][usize::from(pixel)] =
                Pixel { palette: 0, color, priority: 0, char: 0, pos: 0, oam: crate::NO_OBJECT };
        }
    }
}
