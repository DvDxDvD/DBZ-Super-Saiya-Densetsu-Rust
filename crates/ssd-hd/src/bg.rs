//! BG layers of modes 0 and 1: the tile map and character lookups, as the PPU does them.

use crate::{Pixel, Renderer, WIDTH};
use ssd_oracle::LineRegs;

const VRAM_MASK: u16 = (1 << 15) - 1;

/// Bits a pixel takes in a BG's characters.
pub(crate) fn bg_bitplanes(mode: u8, bg: usize) -> usize {
    match (mode, bg) {
        (0, _) | (_, 2 | 3) => 2,
        _ => 4,
    }
}

/// The BG (0 for BG1) a layer of the screen resolver is, if it is one.
pub(crate) fn of_layer(layer: crate::screen::Layer) -> Option<usize> {
    use crate::screen::Layer;
    match layer {
        Layer::Bg1 => Some(0),
        Layer::Bg2 => Some(1),
        Layer::Bg3 => Some(2),
        Layer::Bg4 => Some(3),
        _ => None,
    }
}

/// `(width, height)` of a BG tile in pixels.
fn tile_size(regs: &LineRegs, bg: usize) -> (u16, u16) {
    if regs.bg_large_tiles[bg] { (16, 16) } else { (8, 8) }
}

/// The tile map word at pixel `(x, y)` of the BG's map.
pub(crate) fn map_entry(vram: &[u16], regs: &LineRegs, bg: usize, x: u16, y: u16) -> u16 {
    let (tile_width, tile_height) = tile_size(regs, bg);
    let screen_size = regs.bg_screen_size[bg];
    let wide = matches!(screen_size, 1 | 3);
    let tall = matches!(screen_size, 2 | 3);
    let screen_width = if wide { 64 } else { 32 } * tile_width;
    let screen_height = if tall { 64 } else { 32 } * tile_height;

    let mut base = regs.bg_map_base[bg];
    let mut x = x & (screen_width - 1);
    let mut y = y & (screen_height - 1);

    let single_width = 32 * tile_width;
    let single_height = 32 * tile_height;
    if x >= single_width {
        base = base.wrapping_add(32 * 32);
        x &= single_width - 1;
    }
    if y >= single_height {
        base = base.wrapping_add(if screen_size == 2 { 32 * 32 } else { 2 * 32 * 32 });
        y &= single_height - 1;
    }

    let address = base.wrapping_add(32 * (y / tile_height)).wrapping_add(x / tile_width);
    vram[usize::from(address & VRAM_MASK)]
}

impl Renderer {
    /// Renders the BG layers of the line (the Mode 7 plane in mode 7) into the line buffers,
    /// from pixel `from` on.
    pub(crate) fn render_bg_layers(
        &mut self,
        vram: &[u16],
        regs: &LineRegs,
        scanline: u16,
        from: usize,
    ) {
        if regs.bg_mode == 7 {
            self.render_mode_7(vram, regs, scanline, from);
            return;
        }
        let enabled = |bg: usize| regs.main_bg[bg] || regs.sub_bg[bg];
        let mode_0 = regs.bg_mode == 0;
        let layers = [enabled(0), enabled(1), enabled(2), mode_0 && enabled(3)];
        for (bg, on) in layers.into_iter().enumerate() {
            if on {
                self.render_bg(vram, regs, bg, scanline, from);
            }
        }
    }

    fn render_bg(&mut self, vram: &[u16], regs: &LineRegs, bg: usize, scanline: u16, from: usize) {
        let planes = bg_bitplanes(regs.bg_mode, bg);
        let tile_words: u16 = if planes == 2 { 8 } else { 16 };
        let (tile_width, tile_height) = tile_size(regs, bg);
        let h_scroll = regs.bg_h_scroll[bg];
        let v_scroll = regs.bg_v_scroll[bg];
        let mosaic = regs.mosaic_bg[bg];
        let block = u16::from(regs.mosaic_size) + 1;
        let base_y =
            if mosaic { scanline.wrapping_sub(u16::from(regs.mosaic_v_offset)) } else { scanline };
        let char_base = regs.bg_char_base[bg];

        let y = base_y.wrapping_add(v_scroll);
        if !mosaic {
            // Without the mosaic a map entry and a character row serve the eight pixels of a
            // character's column span at once
            let mut pixel = from;
            while pixel < WIDTH {
                let x = (pixel as u16).wrapping_add(h_scroll);
                let entry = map_entry(vram, regs, bg, x, y);
                let tile_number = entry & 0x3FF;
                let palette = ((entry >> 10) & 0x07) as u8;
                let priority = u8::from(entry & 0x2000 != 0);
                let x_flip = entry & 0x4000 != 0;
                let y_flip = entry & 0x8000 != 0;
                let x_shift = tile_width == 16 && (if x_flip { x % 16 < 8 } else { x % 16 >= 8 });
                let y_shift = tile_height == 16 && (if y_flip { y % 16 < 8 } else { y % 16 >= 8 });
                let tile = tile_number.wrapping_add(u16::from(x_shift)).wrapping_add(if y_shift {
                    16
                } else {
                    0
                });
                let address =
                    usize::from(char_base.wrapping_add(tile.wrapping_mul(tile_words)) & VRAM_MASK);
                let row = if y_flip { 7 - (y % 8) } else { y % 8 };
                let mut words = [0_u16; 2];
                for (n, word) in words.iter_mut().enumerate().take(planes / 2) {
                    *word = vram[(address + usize::from(row) + 8 * n) & 0x7FFF];
                }
                let first = usize::from(x % 8);
                let run = (8 - first).min(WIDTH - pixel);
                let char = address as u16;
                for k in 0..run {
                    let col = if x_flip { 7 - (first + k) } else { first + k } as u8;
                    let bit = 7 - col;
                    let mut color = 0_u8;
                    for (n, &word) in words.iter().enumerate().take(planes / 2) {
                        color |= u8::from((word >> bit) & 1 != 0) << (2 * n);
                        color |= u8::from((word >> (bit + 8)) & 1 != 0) << (2 * n + 1);
                    }
                    self.bg_pixels[bg][pixel + k] = Pixel {
                        palette,
                        color,
                        priority,
                        char,
                        pos: crate::pos(col, row as u8, x_flip, y_flip),
                        oam: crate::NO_OBJECT,
                    };
                }
                pixel += run;
            }
            return;
        }
        for pixel in from..WIDTH {
            let pixel = pixel as u16;
            if mosaic && pixel / block * block != pixel {
                // Not the left pixel of a mosaic block: the one before it
                self.bg_pixels[bg][usize::from(pixel)] = self.bg_pixels[bg][usize::from(pixel - 1)];
                continue;
            }
            let x = pixel.wrapping_add(h_scroll);
            let entry = map_entry(vram, regs, bg, x, y);
            let tile_number = entry & 0x3FF;
            let palette = ((entry >> 10) & 0x07) as u8;
            let priority = u8::from(entry & 0x2000 != 0);
            let x_flip = entry & 0x4000 != 0;
            let y_flip = entry & 0x8000 != 0;

            // 16x16 tiles are four 8x8 characters: the right and lower ones follow by 1 and 16
            let x_shift = tile_width == 16 && (if x_flip { x % 16 < 8 } else { x % 16 >= 8 });
            let y_shift = tile_height == 16 && (if y_flip { y % 16 < 8 } else { y % 16 >= 8 });
            let tile = tile_number.wrapping_add(u16::from(x_shift)).wrapping_add(if y_shift {
                16
            } else {
                0
            });
            let address =
                usize::from(char_base.wrapping_add(tile.wrapping_mul(tile_words)) & VRAM_MASK);

            let row = if y_flip { 7 - (y % 8) } else { y % 8 };
            let col = if x_flip { 7 - (x % 8) } else { x % 8 };
            let bit = 7 - col;
            let mut color = 0_u8;
            for plane in (0..planes).step_by(2) {
                let word = vram[(address + usize::from(row) + 4 * plane) & 0x7FFF];
                color |= u8::from((word >> bit) & 1 != 0) << plane;
                color |= u8::from((word >> (bit + 8)) & 1 != 0) << (plane + 1);
            }
            let char = address as u16;
            let pos = crate::pos(col as u8, row as u8, x_flip, y_flip);
            self.bg_pixels[bg][usize::from(pixel)] =
                Pixel { palette, color, priority, char, pos, oam: crate::NO_OBJECT };
        }
    }
}
