//! Objects: the PPU's evaluation (up to 32 sprites a line, taken in OAM order from a start index)
//! and tile fetch (up to 34 tiles a line, the sprites last to first) into the line buffer, from
//! OAM and VRAM and the object registers the log records for the line.

use crate::{Memories, Pixel, Renderer, WIDTH};
use ssd_oracle::SpriteLine;

const MAX_SPRITES_PER_LINE: usize = 32;
const MAX_TILES_PER_LINE: usize = 34;

/// `(small, large)` sprite sizes as `(width, height)` for OBSEL's size field.
fn sizes(index: u8) -> ((u16, u16), (u16, u16)) {
    match index {
        0 => ((8, 8), (16, 16)),
        1 => ((8, 8), (32, 32)),
        2 => ((8, 8), (64, 64)),
        3 => ((16, 16), (32, 32)),
        4 => ((16, 16), (64, 64)),
        5 => ((32, 32), (64, 64)),
        6 => ((16, 32), (32, 64)),
        _ => ((16, 32), (32, 32)),
    }
}

/// Whether the sprite at `y` with `height` covers `scanline`, in 8-bit arithmetic with wrapping.
fn overlaps(y: u8, height: u16, scanline: u16) -> bool {
    let scanline = scanline as u8;
    let bottom = y.wrapping_add(height as u8);
    if bottom > y { (y..bottom).contains(&scanline) } else { scanline >= y || scanline < bottom }
}

/// One OAM entry.
struct Sprite {
    x: u16,
    y: u8,
    tile: u16,
    palette: u8,
    priority: u8,
    x_flip: bool,
    y_flip: bool,
    large: bool,
}

fn sprite(oam: (&[u8], &[u8]), index: u8) -> Sprite {
    let low = &oam.0[usize::from(index) * 4..usize::from(index) * 4 + 4];
    let high = oam.1[usize::from(index >> 2)] >> (2 * (index & 3));
    let attributes = low[3];
    Sprite {
        x: u16::from(low[0]) | (u16::from(high & 1) << 8),
        y: low[1],
        tile: u16::from(low[2]) | (u16::from(attributes & 1) << 8),
        palette: (attributes >> 1) & 0x07,
        priority: (attributes >> 4) & 0x03,
        x_flip: attributes & 0x40 != 0,
        y_flip: attributes & 0x80 != 0,
        large: high & 2 != 0,
    }
}

/// A fetched tile of eight pixels, from the character at VRAM word address `char`.
struct Tile {
    x: i32,
    palette: u8,
    priority: u8,
    char: u16,
    /// The OAM entry, and the character's row and flips for [`crate::pos`].
    oam: u8,
    row: u8,
    x_flip: bool,
    y_flip: bool,
    colors: [u8; 8],
}

impl Renderer {
    /// Rebuilds the object line buffer the way the PPU did at the end of the scanline `sp` names.
    ///
    /// `base` is the window position of the buffer's first pixel: 0 for the PPU's own frame. The
    /// PPU takes the sprites and counts the tiles by where they lie on the screen, so the window
    /// positions decide which are taken, whatever part of the frame the buffer shows: a wide view's
    /// chunk gives the window's positions and draws the parts that fall in its buffer.
    pub(crate) fn build_sprite_line(
        &mut self,
        memories: &Memories<'_>,
        sp: &SpriteLine,
        base: i32,
    ) {
        self.obj_pixels.fill(Pixel::TRANSPARENT);
        if sp.blank || sp.fetch_blanked {
            return;
        }

        // Evaluation: the first 32 sprites with pixels on the screen, on the line, from the start
        // index, over 128 entries
        let (small, large) = sizes(sp.scan_size);
        let mut scanned: Vec<u8> = Vec::with_capacity(MAX_SPRITES_PER_LINE);
        let mut index = sp.start_oam_idx & 0x7F;
        for _ in 0..128 {
            let sprite = sprite(memories.oam, index);
            let (width, height) = if sprite.large { large } else { small };
            let next = (index + 1) & 0x7F;
            let on_line = overlaps(sprite.y, height, sp.eval_line);
            let x = signed_x(sprite.x);
            let on_screen = x < WIDTH as i32 && x + i32::from(width) > 0;
            if on_line && on_screen {
                if scanned.len() == MAX_SPRITES_PER_LINE {
                    break;
                }
                scanned.push(index);
            }
            index = next;
        }

        // Tile fetch: the sprites last to first, each left to right. The tiles on the screen are
        // counted (34 a line); the tiles outside it are drawn only when the buffer shows them
        let (small, large) = sizes(sp.fetch_size);
        let mut tiles: Vec<Tile> = Vec::with_capacity(MAX_TILES_PER_LINE);
        let mut counted = 0;
        'sprites: for &index in scanned.iter().rev() {
            let sprite = sprite(memories.oam, index);
            let (width, height) = if sprite.large { large } else { small };
            if !overlaps(sprite.y, height, sp.eval_line) {
                // Its Y changed between the scan and the fetch (or its size did)
                continue;
            }
            let row = (sp.eval_line as u8).wrapping_sub(sprite.y) & ((height - 1) as u8);
            let line = if sprite.y_flip { height as u8 - 1 - row } else { row };
            let tile_y = u16::from(line / 8);

            for tile_x in 0..width / 8 {
                let sprite_x = signed_x(sprite.x);
                let x = if sprite.x_flip {
                    sprite_x + i32::from(width - 8) - 8 * i32::from(tile_x)
                } else {
                    sprite_x + 8 * i32::from(tile_x)
                };
                // On the screen: [-8, 256), a tile that shows a pixel of it
                let on_screen = (-8..WIDTH as i32).contains(&x);
                let in_buffer = x + 8 > base && x < base + WIDTH as i32;
                if !on_screen && !in_buffer {
                    continue;
                }
                if on_screen {
                    if counted == MAX_TILES_PER_LINE {
                        break 'sprites;
                    }
                    counted += 1;
                }

                // Overflows in a large sprite's tile numbers do not carry to the next nibble
                let mut number = sprite.tile;
                number = (number & !0xF) | (number.wrapping_add(tile_x) & 0xF);
                number = (number & !0xF0) | (number.wrapping_add(tile_y << 4) & 0xF0);
                let char_base = sp
                    .fetch_base
                    .wrapping_add(u16::from(number & 0x100 != 0) * (256 * 16 + sp.fetch_gap));
                let address = usize::from(char_base.wrapping_add((number & 0xFF) * 16) & 0x7FFF);
                let data = &memories.vram[address..address + 16];
                let tile_row = usize::from(line % 8);

                let mut colors = [0_u8; 8];
                for col in 0..8_u8 {
                    let bit = 7 - col;
                    let mut color = 0_u8;
                    for plane in 0..2 {
                        let word = data[tile_row + 8 * plane];
                        color |= u8::from((word >> bit) & 1 != 0) << (2 * plane);
                        color |= u8::from((word >> (bit + 8)) & 1 != 0) << (2 * plane + 1);
                    }
                    colors[usize::from(if sprite.x_flip { 7 - col } else { col })] = color;
                }
                let char = address as u16;
                tiles.push(Tile {
                    x,
                    palette: sprite.palette,
                    priority: sprite.priority,
                    char,
                    oam: index,
                    row: tile_row as u8,
                    x_flip: sprite.x_flip,
                    y_flip: sprite.y_flip,
                    colors,
                });
            }
        }

        // The tiles go into the line buffer in the order they were fetched
        for tile in &tiles {
            for dx in 0..8_i32 {
                let pixel = tile.x + dx - base;
                let color = tile.colors[dx as usize];
                let hidden = self.hidden_objects >> tile.oam & 1 != 0;
                if (0..WIDTH as i32).contains(&pixel) && color != 0 && !hidden {
                    // The character's column: the sprite's flip undone
                    let col = if tile.x_flip { 7 - dx } else { dx } as u8;
                    self.obj_pixels[pixel as usize] = Pixel {
                        palette: tile.palette,
                        color,
                        priority: tile.priority,
                        char: tile.char,
                        pos: crate::pos(col, tile.row, tile.x_flip, tile.y_flip),
                        oam: tile.oam,
                    };
                }
            }
        }
    }
}

/// The 9-bit X of an object as a signed position: -256 to 255, the screen's 0 to 255 among them.
fn signed_x(x: u16) -> i32 {
    let x = i32::from(x);
    if x >= 256 { x - 512 } else { x }
}
