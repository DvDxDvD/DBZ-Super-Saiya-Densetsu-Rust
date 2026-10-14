//! Palette variants: art made for one palette drawn in another.
//!
//! The game draws colour variants of one enemy (and the same face in several rows) from the same
//! tiles in different palettes, and the art was made from one of them. Where the art entry matches
//! by its tiles or shape but not by its palette, the art is recoloured: every pixel is placed
//! between the colours of the palette it was made for (on the segment between the two nearest
//! ones), and takes the same place between the current palette's colours, with the art's own
//! shading (its luminance against that of the place it came from) kept.

use super::resample::Image;
use crate::art_index::Palette;

/// How many 8-bit levels a colour of the current palette may differ from the art's palette in a
/// channel, and still be the same colour (the importer and the PPU's 5 to 8 bit steps differ by
/// one).
const SAME: u8 = 3;

/// The softening of the luminance ratio, so that a very dark colour does not scale wildly.
const SOFT: u32 = 12;

/// How far (summed channel differences) a colour of the current palette may be from the art's
/// one before the art's own deviation from its palette colour no longer carries over.
const KEEP_RESIDUAL: i32 = 192;

/// Art to draw in the colours of `row`: the palette the art was made for is the art entry's own,
/// or (`derive`, for art that has none) read from the pose being drawn ([`derive_palette`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Variant {
    pub row: [[u8; 3]; 16],
    pub derive: bool,
}

/// The palette art of a pose was made for, read back from the art: the colour of each palette
/// index is the mean colour of the art over the pixels of the pose that have that index (`arr`,
/// `width` x `height` pixels of the original; the art is `scale` times larger). The art is the
/// pose redrawn, so each index's pixels are the colours of one part of it.
pub(crate) fn derive_palette(
    image: &Image,
    arr: &[u8],
    width: usize,
    height: usize,
    scale: usize,
) -> Palette {
    let mut sums = [[0_u64; 3]; 16];
    let mut counts = [0_u64; 16];
    for y in 0..height.min(image.height / scale) {
        for x in 0..width.min(image.width / scale) {
            let index = usize::from(arr[y * width + x] & 15);
            if index == 0 {
                continue;
            }
            for sy in 0..scale {
                for sx in 0..scale {
                    let at = ((y * scale + sy) * image.width + x * scale + sx) * 4;
                    let pixel = &image.rgba[at..at + 4];
                    if pixel[3] >= 128 {
                        counts[index] += 1;
                        for c in 0..3 {
                            sums[index][c] += u64::from(pixel[c]);
                        }
                    }
                }
            }
        }
    }
    let mut palette = Palette::default();
    for index in 1..16 {
        if counts[index] > 0 {
            palette.mask |= 1 << index;
            palette.colors[index] = sums[index].map(|sum| (sum / counts[index]) as u8);
        }
    }
    palette
}

/// Whether some colour the art uses is another colour in `row`.
#[must_use]
pub(crate) fn differs(source: &Palette, row: &[[u8; 3]; 16]) -> bool {
    source.mask != 0
        && (1..16).any(|index| {
            source.mask & (1 << index) != 0
                && source.colors[index].iter().zip(&row[index]).any(|(a, b)| a.abs_diff(*b) > SAME)
        })
}

fn luma(color: [i32; 3]) -> i32 {
    (color[0] * 299 + color[1] * 587 + color[2] * 114) / 1000
}

fn dist2(a: [i32; 3], b: [i32; 3]) -> i32 {
    (0..3).map(|c| (a[c] - b[c]) * (a[c] - b[c])).sum()
}

/// The colours the art's pixels are looked up in: index, colour in the art's palette, colour in
/// the current one.
fn colours(source: &Palette, row: &[[u8; 3]; 16]) -> Vec<([i32; 3], [i32; 3])> {
    (1..16)
        .filter(|&index| source.mask & (1 << index) != 0)
        .map(|index| (source.colors[index].map(i32::from), row[index].map(i32::from)))
        .collect()
}

/// The colour `pixel` (of the art made for `colours`' first colours) becomes in the second ones.
fn map_pixel(pixel: [i32; 3], colours: &[([i32; 3], [i32; 3])]) -> [u8; 3] {
    // The two nearest colours of the art's palette
    let (mut first, mut second) = (0, usize::MAX);
    let mut best = (i32::MAX, i32::MAX);
    for (at, (source, _)) in colours.iter().enumerate() {
        let d = dist2(pixel, *source);
        if d < best.0 {
            best = (d, best.0);
            second = first;
            first = at;
        } else if d < best.1 {
            best.1 = d;
            second = at;
        }
    }
    let (a_source, a_target) = colours[first];
    // The place on the segment between the two (0 at the nearest), in 1/256
    let (mut base_source, mut base_target) = (a_source, a_target);
    if second != usize::MAX {
        let (b_source, b_target) = colours[second];
        let axis: [i32; 3] = [0, 1, 2].map(|c| b_source[c] - a_source[c]);
        let length = axis.iter().map(|v| v * v).sum::<i32>();
        if length > 0 {
            let along: i32 = (0..3).map(|c| (pixel[c] - a_source[c]) * axis[c]).sum();
            let t = (along * 256 / length).clamp(0, 256);
            base_source = [0, 1, 2].map(|c| a_source[c] + (b_source[c] - a_source[c]) * t / 256);
            base_target = [0, 1, 2].map(|c| a_target[c] + (b_target[c] - a_target[c]) * t / 256);
        }
    }
    let shift = (0..3).map(|c| (base_target[c] - base_source[c]).abs()).sum::<i32>();
    let softly = |value: i32| value.max(0) + SOFT as i32;
    let ratio = softly(luma(pixel)) * 256 / softly(luma(base_source));
    // The pixel's own deviation from its palette's colour carries over while the palettes are
    // alike (and is all there is when they are the same)
    let residual = (KEEP_RESIDUAL - shift).clamp(0, KEEP_RESIDUAL);
    [0, 1, 2].map(|c| {
        let shaded = base_target[c] * ratio / 256;
        let own = pixel[c] - base_source[c] * ratio / 256;
        (shaded + own * residual / KEEP_RESIDUAL).clamp(0, 255) as u8
    })
}

/// `image`, made for `source`, drawn in the colours of `row`: opaque colours recoloured, alpha and
/// size kept. The image is returned as it is when the palettes do not differ.
#[must_use]
pub(crate) fn recolour(image: &Image, source: &Palette, row: &[[u8; 3]; 16]) -> Image {
    let colours = colours(source, row);
    if colours.is_empty() || !differs(source, row) {
        return image.clone();
    }
    let mut rgba = image.rgba.clone();
    // The art has few distinct colours at the scale it was made, many after the filter: the
    // last answer is remembered, which is most of a flat area
    let mut last: Option<([u8; 3], [u8; 3])> = None;
    for pixel in rgba.as_chunks_mut::<4>().0 {
        if pixel[3] == 0 {
            continue;
        }
        let colour = [pixel[0], pixel[1], pixel[2]];
        let mapped = match last {
            Some((from, to)) if from == colour => to,
            _ => {
                let to = map_pixel(colour.map(i32::from), &colours);
                last = Some((colour, to));
                to
            }
        };
        pixel[..3].copy_from_slice(&mapped);
    }
    Image { width: image.width, height: image.height, rgba, luma_ref: image.luma_ref }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(colours: &[(usize, [u8; 3])]) -> Palette {
        let mut palette = Palette::default();
        for &(index, colour) in colours {
            palette.colors[index] = colour;
            palette.mask |= 1 << index;
        }
        palette
    }

    fn row(colours: &[(usize, [u8; 3])]) -> [[u8; 3]; 16] {
        let mut row = [[0_u8; 3]; 16];
        for &(index, colour) in colours {
            row[index] = colour;
        }
        row
    }

    fn image_of(pixels: &[[u8; 4]]) -> Image {
        Image::new(pixels.len(), 1, pixels.concat())
    }

    const GREEN: [u8; 3] = [90, 200, 80];
    const OUTLINE: [u8; 3] = [20, 30, 20];
    const BLUE: [u8; 3] = [70, 90, 220];
    const DARK_BLUE: [u8; 3] = [10, 10, 60];

    #[test]
    fn the_same_palette_leaves_the_art_alone_and_one_colour_step_is_the_same_colour() {
        let source = palette(&[(1, GREEN), (2, OUTLINE)]);
        let image = image_of(&[[91, 201, 79, 255], [5, 6, 7, 255]]);
        let same = row(&[(1, GREEN), (2, OUTLINE)]);
        assert!(!differs(&source, &same));
        assert_eq!(recolour(&image, &source, &same), image);
        let near = row(&[(1, [92, 199, 80]), (2, OUTLINE)]);
        assert!(!differs(&source, &near), "a step of the colour conversion is not a variant");
        assert!(differs(&source, &row(&[(1, BLUE), (2, OUTLINE)])));
        assert!(
            !differs(&Palette::default(), &row(&[(1, BLUE)])),
            "no palette, nothing to compare"
        );
    }

    #[test]
    fn a_palette_colour_becomes_the_colour_of_the_same_index_in_the_current_palette() {
        let source = palette(&[(1, GREEN), (2, OUTLINE)]);
        let target = row(&[(1, BLUE), (2, DARK_BLUE)]);
        let image = image_of(&[[GREEN[0], GREEN[1], GREEN[2], 255], [20, 30, 20, 255]]);
        let out = recolour(&image, &source, &target);
        assert_eq!(out.rgba[..3], BLUE);
        assert_eq!(out.rgba[4..7], DARK_BLUE);
        assert_eq!((out.width, out.height), (2, 1));
    }

    #[test]
    fn the_arts_shading_is_kept_and_alpha_is_untouched() {
        let source = palette(&[(1, GREEN), (2, OUTLINE)]);
        let target = row(&[(1, BLUE), (2, DARK_BLUE)]);
        let light = [135, 255, 120, 255]; // GREEN, lit
        let shade = [45, 100, 40, 255]; // GREEN, shaded
        let faded = [90, 200, 80, 100]; // GREEN, half transparent
        let clear = [1, 2, 3, 0];
        let out = recolour(&image_of(&[light, shade, faded, clear]), &source, &target);
        let luma_of = |at: usize| luma([0, 1, 2].map(|c| i32::from(out.rgba[at * 4 + c])));
        assert!(luma_of(0) > luma(BLUE.map(i32::from)), "lit stays lighter");
        assert!(luma_of(1) < luma(BLUE.map(i32::from)), "shaded stays darker");
        // Still blue in hue: the blue channel leads in all of them
        for at in 0..3 {
            let [r, g, b] = [0, 1, 2].map(|c| out.rgba[at * 4 + c]);
            assert!(b > r && b > g, "pixel {at} is blue: {r} {g} {b}");
        }
        assert_eq!(out.rgba[4 * 2 + 3], 100, "alpha kept");
        assert_eq!(out.rgba[4 * 3..4 * 3 + 4], [1, 2, 3, 0], "a transparent pixel is untouched");
    }

    #[test]
    fn a_colour_between_two_palette_colours_lands_between_theirs() {
        let source = palette(&[(1, [200, 0, 0]), (2, [0, 0, 200])]);
        let target = row(&[(1, [0, 200, 0]), (2, [200, 200, 0])]);
        // Halfway between the red and the blue
        let out = recolour(&image_of(&[[100, 0, 100, 255]]), &source, &target);
        let [r, g, b] = [out.rgba[0], out.rgba[1], out.rgba[2]];
        assert!(r > 60 && r < 140, "halfway from green to yellow in red: {r}");
        assert!(g > 150, "green stays: {g}");
        assert!(b < 40, "no blue: {b}");
    }

    #[test]
    fn the_palette_of_art_is_the_mean_colour_over_the_pixels_of_each_index() {
        // A 2 x 1 pose of indexes 1 and 2, the art at 2x: index 1 is red and a darker red,
        // index 2 is blue; the pose's third pixel column does not exist
        let red = [200, 0, 0, 255];
        let dark = [100, 0, 0, 255];
        let blue = [0, 0, 200, 255];
        let pixels = [red, dark, blue, blue, red, dark, blue, blue];
        let image = Image::new(4, 2, pixels.concat());
        let palette = derive_palette(&image, &[1, 2], 2, 1, 2);
        assert_eq!(palette.mask, 0b110);
        assert_eq!(palette.colors[1], [150, 0, 0]);
        assert_eq!(palette.colors[2], [0, 0, 200]);
    }

    #[test]
    fn nearly_alike_palettes_give_nearly_the_art() {
        let source = palette(&[(1, GREEN), (2, OUTLINE)]);
        // A palette a few steps darker (a fade): a pixel the palette does not hold (a white
        // highlight) stays close to itself
        let target = row(&[(1, [80, 190, 70]), (2, OUTLINE)]);
        let out = recolour(&image_of(&[[250, 255, 250, 255]]), &source, &target);
        for c in 0..3 {
            assert!(out.rgba[c].abs_diff([250, 255, 250][c]) < 40, "channel {c}: {:?}", out.rgba);
        }
    }
}
