//! The command screen's ground shadow: a BG1 layer of one dark flat ellipse under the fighters,
//! drawn by the game in square pixels. It is found by the BG1 pixels' own colour (whatever is in
//! front of them: a fighter's feet), by the ellipse rule of the sprites' shadows, and drawn again as
//! a smooth soft ellipse over the HD picture ([`super::draw::paint`]), with the art of the
//! background under it instead of the layer's black pixels.

use super::ArtPixel;
use super::draw::{Ellipse, SHADOW_MARGIN, Shape, at_frame, paint};
use super::poses::{SHADOW_LUMA, SHADOW_MIN, ellipse, rgb555};
use crate::bgart::{Kind, NO_COLOUR};
use crate::{Frame, NO_OBJECT};

/// The most pixels of a shadow: a bigger dark layer is not one.
const MAX_PIXELS: usize = 4000;

/// A ground shadow of BG1: its box in the frame, its silhouette, and the frame's pixels of it
/// that BG1 itself shows.
pub(crate) struct BgShadow {
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    arr: Vec<u8>,
    /// For each cell of the box, the index of the frame's pixel when it is part of the silhouette
    /// and BG1 is what the frame shows there.
    shown: Vec<Option<usize>>,
}

/// Finds the ground shadows among the BG1 pixels of the frame, and makes the art of the
/// background draw under the pixels BG1 shows of them ([`Kind::Under`] becomes [`Kind::Layer`]).
pub(crate) fn find(frame: &Frame, pixels: &mut [ArtPixel]) -> Vec<BgShadow> {
    let dark = |pixel: &ArtPixel| {
        pixel.bg.shadow != NO_COLOUR && {
            let [r, g, b] = rgb555(pixel.bg.shadow);
            (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000 < SHADOW_LUMA
        }
    };
    let (width, height) = (frame.width, frame.height);
    let mut seen = vec![false; pixels.len()];
    let mut out = Vec::new();
    for start in 0..pixels.len() {
        if seen[start] || !dark(&pixels[start]) {
            continue;
        }
        // The four-connected pixels of the same colour
        let colour = pixels[start].bg.shadow;
        let mut stack = vec![start];
        let mut cells = Vec::new();
        seen[start] = true;
        while let Some(at) = stack.pop() {
            cells.push(at);
            let (x, y) = (at % width, at / width);
            let mut near = Vec::with_capacity(4);
            if x > 0 {
                near.push(at - 1);
            }
            if x + 1 < width {
                near.push(at + 1);
            }
            if y > 0 {
                near.push(at - width);
            }
            if y + 1 < height {
                near.push(at + width);
            }
            for other in near {
                if !seen[other] && pixels[other].bg.shadow == colour {
                    seen[other] = true;
                    stack.push(other);
                }
            }
            if cells.len() > MAX_PIXELS {
                break;
            }
        }
        if cells.len() < SHADOW_MIN || cells.len() > MAX_PIXELS {
            continue;
        }
        if !cells.iter().any(|&at| pixels[at].bg.kind == Kind::Under) {
            continue;
        }
        let xs = cells.iter().map(|at| at % width);
        let ys = cells.iter().map(|at| at / width);
        let (left, right) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0) + 1);
        let (top, bottom) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0) + 1);
        let (w, h) = (right - left, bottom - top);
        let mut arr = vec![0_u8; w * h];
        let mut shown = vec![None; w * h];
        for &at in &cells {
            let cell = (at / width - top) * w + at % width - left;
            arr[cell] = 1;
            if pixels[at].bg.kind == Kind::Under && pixels[at].oam == NO_OBJECT {
                shown[cell] = Some(at);
            }
        }
        if !ellipse(&arr, w, h) {
            continue;
        }
        for at in shown.iter().flatten() {
            pixels[*at].bg.kind = Kind::Layer;
        }
        out.push(BgShadow { x0: left as i32, y0: top as i32, w, h, arr, shown });
    }
    out
}

/// Draws the shadows `found` over `rgba`, under the poses (`others`: the sprites drawn as art, whose
/// pixels are the shadow's where its silhouette is, since those poses are drawn after it).
pub(crate) fn draw(
    rgba: &mut [u8],
    scale: usize,
    frame: &Frame,
    pixels: &[ArtPixel],
    found: &[BgShadow],
    others: u128,
    clip: (i32, i32, i32, i32),
) {
    let margin = SHADOW_MARGIN;
    for shadow in found {
        let (w, h) = (shadow.w as i32 + 2 * margin, shadow.h as i32 + 2 * margin);
        let (x0, y0) = (shadow.x0 - margin, shadow.y0 - margin);
        let smooth = Ellipse::fitted(shadow.w, shadow.h, &shadow.arr, margin);
        let mut grid: Vec<Option<[u8; 3]>> = vec![None; (w * h) as usize];
        let mut solid = vec![0.0_f32; (w * h) as usize];
        let mut covered = Vec::new();
        let mut known: Option<[u8; 3]> = None;
        for y in 0..h {
            for x in 0..w {
                let at = (y * w + x) as usize;
                let Some((here, index)) = at_frame(frame, pixels, clip, x0 + x, y0 + y) else {
                    continue;
                };
                let (cx, cy) = (x - margin, y - margin);
                let cell = (cx >= 0 && cy >= 0 && cx < shadow.w as i32 && cy < shadow.h as i32)
                    .then(|| cy as usize * shadow.w + cx as usize);
                let inside = cell.is_some_and(|cell| shadow.arr[cell] != 0);
                let shown = cell.is_some_and(|cell| shadow.shown[cell] == Some(index));
                if shown {
                    let c = frame.pixels[index];
                    grid[at] = Some([c.r, c.g, c.b]);
                    known.get_or_insert([c.r, c.g, c.b]);
                    solid[at] = 1.0;
                } else if (inside || smooth.is_some())
                    && here.oam != NO_OBJECT
                    && others >> here.oam & 1 != 0
                {
                    // Under a fighter drawn from art: the shadow is there
                    covered.push(at);
                    solid[at] = f32::from(u8::from(inside));
                } else if !inside
                    && here.oam == NO_OBJECT
                    && here.bg.battle
                    && here.bg.shadow == NO_COLOUR
                {
                    // Beside the silhouette, over the background's art: the soft edge
                    grid[at] = Some([0; 3]);
                    covered.push(at);
                }
            }
        }
        let Some(colour) = known else { continue };
        for at in covered {
            grid[at] = Some(colour);
        }
        paint(
            rgba,
            scale,
            frame,
            pixels,
            clip,
            &Shape { x0, y0, w, h, grid, solid, smooth, colour },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_oracle::Color;

    /// A 64 x 32 frame whose BG1 shows `rows` (a run of pixels per row, as `(from, to)` from
    /// column 10) in the dark colour 0, with BG1 winning each of them.
    fn frame_of(rows: &[(usize, usize)]) -> (Frame, Vec<ArtPixel>) {
        let frame = Frame {
            width: 64,
            height: 32,
            pixels: vec![Color::BLACK; 64 * 32],
            sources: Vec::new(),
        };
        let mut pixels = vec![ArtPixel::NONE; 64 * 32];
        for (row, &(from, to)) in rows.iter().enumerate() {
            for x in from..to {
                let pixel = &mut pixels[(10 + row) * 64 + 10 + x];
                pixel.bg.shadow = 0;
                pixel.bg.kind = Kind::Under;
            }
        }
        (frame, pixels)
    }

    #[test]
    fn a_dark_ellipse_of_bg1_is_a_shadow_and_the_art_draws_under_it() {
        let (frame, mut pixels) = frame_of(&[(4, 12), (1, 15), (0, 16), (0, 16), (1, 15), (4, 12)]);
        let found = find(&frame, &mut pixels);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].x0, found[0].y0, found[0].w, found[0].h), (10, 10, 16, 6));
        let under = pixels.iter().filter(|p| p.bg.kind == Kind::Layer).count();
        assert_eq!(under, 8 + 14 + 16 + 16 + 14 + 8, "every pixel BG1 shows of it");
    }

    #[test]
    fn a_pixel_in_front_of_the_shadow_is_still_part_of_its_shape() {
        let (frame, mut pixels) = frame_of(&[(4, 12), (1, 15), (0, 16), (0, 16), (1, 15), (4, 12)]);
        // A foot (an object) covers three pixels of the widest row: the silhouette stays whole
        for x in 20..23 {
            let pixel = &mut pixels[12 * 64 + x];
            pixel.oam = 3;
            pixel.bg.kind = Kind::Object;
        }
        let found = find(&frame, &mut pixels);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].arr.iter().filter(|&&c| c != 0).count(), 76);
        assert_eq!(found[0].shown.iter().flatten().count(), 73, "the foot's pixels are not BG1's");
        assert_eq!(pixels[12 * 64 + 21].bg.kind, Kind::Object, "left to the pose's art");
    }

    #[test]
    fn dark_layers_that_are_not_ellipses_are_left_alone() {
        // A bar with square ends, and a tall block
        let (frame, mut pixels) = frame_of(&[(0, 16); 4]);
        assert!(find(&frame, &mut pixels).is_empty());
        let (frame, mut pixels) = frame_of(&[(0, 4); 12]);
        assert!(find(&frame, &mut pixels).is_empty());
        assert!(pixels.iter().all(|p| p.bg.kind != Kind::Layer));
    }
}
