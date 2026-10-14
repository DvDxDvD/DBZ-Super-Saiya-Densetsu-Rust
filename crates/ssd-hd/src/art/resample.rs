//! Resizing the pack's art to the scale of the picture, in integer arithmetic (the output does
//! not depend on the machine): a box filter when the art is shrunk, bilinear when it is enlarged.
//! Colours are weighted by alpha, so a transparent pixel's colour never bleeds into an edge.

/// RGBA pixels, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// For luma art: the luminance (0 to 255) that counts as full brightness, chosen so that the
    /// art, lit by the original's colour and capped at it, keeps [`LUMA_KEPT`] of that colour on
    /// average (alpha-weighted): a luma effect is the original's colour with the art's shading,
    /// not a darker version of it. 255 for the others.
    pub luma_ref: u8,
}

/// The percentage of the original's colour that luma art keeps on average (see
/// [`Image::luma_ref`]).
const LUMA_KEPT: u64 = 97;

impl Image {
    /// The luminance of pixel `at` (Rec. 601 weights, 0 to 255).
    #[must_use]
    pub fn luma(&self, at: usize) -> u32 {
        luma(&self.rgba[at * 4..at * 4 + 3])
    }

    /// An image of `rgba` pixels; `luma_ref` is measured from them.
    #[must_use]
    pub fn new(width: usize, height: usize, rgba: Vec<u8>) -> Self {
        let mut histogram = [0_u64; 256];
        for pixel in rgba.as_chunks::<4>().0 {
            histogram[luma(pixel) as usize] += u64::from(pixel[3]);
        }
        Self { width, height, rgba, luma_ref: luma_reference(&histogram) }
    }
}

/// The largest reference level `r` (1 to 255) for which the pixels of a `histogram` of luminances
/// (weighted by alpha), each lit `min(level / r, 1)`, keep [`LUMA_KEPT`] of full brightness on
/// average; 255 for an image with nothing opaque.
fn luma_reference(histogram: &[u64; 256]) -> u8 {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return 255;
    }
    // Levels below r give level / r, the others 1: the sums are kept as running prefixes
    let weighted: Vec<u64> = histogram
        .iter()
        .enumerate()
        .scan(0_u64, |sum, (level, &count)| {
            *sum += count * level as u64;
            Some(*sum)
        })
        .collect();
    let counted: Vec<u64> = histogram
        .iter()
        .scan(0_u64, |sum, &count| {
            *sum += count;
            Some(*sum)
        })
        .collect();
    for reference in (1..=255_usize).rev() {
        let (below_weighted, below_count) = (weighted[reference - 1], counted[reference - 1]);
        // kept = below_weighted / reference + (total - below_count), against LUMA_KEPT % of total
        let kept = below_weighted + (total - below_count) * reference as u64;
        if 100 * kept >= LUMA_KEPT * total * reference as u64 {
            return reference as u8;
        }
    }
    1
}

fn luma(rgb: &[u8]) -> u32 {
    (u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114) / 1000
}

/// The taps of one axis: for each destination index, up to two... or more source indexes with
/// integer weights that sum to `total`.
fn axis(source: usize, dest: usize) -> (Vec<Vec<(usize, u32)>>, u64) {
    let (s, d) = (source as u64, dest as u64);
    let mut taps = Vec::with_capacity(dest);
    if dest <= source {
        // A box filter: the destination pixel covers [i * s, (i + 1) * s) in units of 1/d of a
        // source pixel; source pixel k covers [k * d, (k + 1) * d)
        for i in 0..d {
            let (from, to) = (i * s, (i + 1) * s);
            let mut list = Vec::new();
            let mut k = from / d;
            while k * d < to {
                let overlap = to.min((k + 1) * d) - from.max(k * d);
                if overlap > 0 {
                    list.push((k as usize, overlap as u32));
                }
                k += 1;
            }
            taps.push(list);
        }
        (taps, s)
    } else {
        // Bilinear, centres aligned: the source position of the destination pixel's centre is
        // ((2i + 1) s - d) / (2d), in source pixels
        for i in 0..d {
            let position = ((2 * i + 1) * s) as i64 - d as i64;
            let denominator = (2 * d) as i64;
            let first = position.div_euclid(denominator);
            let fraction = position.rem_euclid(denominator) as u32;
            let clamp = |k: i64| k.clamp(0, s as i64 - 1) as usize;
            taps.push(vec![
                (clamp(first), denominator as u32 - fraction),
                (clamp(first + 1), fraction),
            ]);
        }
        (taps, 2 * d)
    }
}

/// `image` resized to `width` x `height`.
///
/// # Panics
///
/// A size of 0.
#[must_use]
pub fn resize(image: &Image, width: usize, height: usize) -> Image {
    assert!(width > 0 && height > 0 && image.width > 0 && image.height > 0);
    if (width, height) == (image.width, image.height) {
        return image.clone();
    }
    let (columns, x_total) = axis(image.width, width);
    let (rows, y_total) = axis(image.height, height);
    let total = x_total * y_total;
    // The rows across first, in units of `x_total`: premultiplied colour and alpha
    let mut across = vec![[0_u64; 4]; image.height * width];
    for y in 0..image.height {
        for (x, taps) in columns.iter().enumerate() {
            let mut sum = [0_u64; 4];
            for &(k, weight) in taps {
                let at = (y * image.width + k) * 4;
                let alpha = u64::from(image.rgba[at + 3]);
                let weight = u64::from(weight);
                for (total, &value) in sum.iter_mut().zip(&image.rgba[at..at + 3]) {
                    *total += weight * alpha * u64::from(value);
                }
                sum[3] += weight * alpha;
            }
            across[y * width + x] = sum;
        }
    }
    let mut out = vec![0_u8; width * height * 4];
    for (y, taps) in rows.iter().enumerate() {
        for x in 0..width {
            let mut sum = [0_u64; 4];
            for &(k, weight) in taps {
                let sample = across[k * width + x];
                for (total, value) in sum.iter_mut().zip(sample) {
                    *total += u64::from(weight) * value;
                }
            }
            let at = (y * width + x) * 4;
            for (value, &channel) in out[at..at + 3].iter_mut().zip(&sum[..3]) {
                *value = (channel + sum[3] / 2).checked_div(sum[3]).unwrap_or(0).min(255) as u8;
            }
            if sum[3] > 0 {
                out[at + 3] = ((sum[3] + total / 2) / total).min(255) as u8;
            }
        }
    }
    Image { width, height, rgba: out, luma_ref: image.luma_ref }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, pixel: [u8; 4]) -> Image {
        Image::new(width, height, pixel.repeat(width * height))
    }

    #[test]
    fn the_same_size_is_the_same_image_and_a_flat_colour_stays_flat() {
        let image = solid(8, 8, [10, 20, 30, 255]);
        assert_eq!(resize(&image, 8, 8), image);
        for (w, h) in [(4, 4), (2, 6), (12, 12), (16, 8), (10, 10)] {
            let out = resize(&image, w, h);
            assert_eq!((out.width, out.height), (w, h));
            assert!(out.rgba.as_chunks::<4>().0.iter().all(|p| *p == [10, 20, 30, 255]), "{w}x{h}");
        }
    }

    #[test]
    fn halving_averages_two_by_two_blocks() {
        let mut rgba = Vec::new();
        for pixel in [[0, 0, 0, 255], [200, 100, 50, 255], [200, 100, 50, 255], [0, 0, 0, 255]] {
            rgba.extend_from_slice(&pixel);
        }
        let out = resize(&Image::new(2, 2, rgba), 1, 1);
        assert_eq!(out.rgba, [100, 50, 25, 255]);
    }

    #[test]
    fn a_transparent_pixel_does_not_tint_its_neighbour() {
        let rgba = [[255, 0, 0, 255], [0, 255, 0, 0]].concat();
        let out = resize(&Image::new(2, 1, rgba), 1, 1);
        assert_eq!(out.rgba, [255, 0, 0, 128]);
    }

    #[test]
    fn enlarging_by_four_over_three_keeps_the_edges_and_blends_between() {
        let rgba = [[0, 0, 0, 255], [255, 255, 255, 255]].concat();
        let out = resize(&Image::new(2, 1, rgba), 8, 1);
        assert_eq!(out.rgba[0], 0);
        assert_eq!(out.rgba[7 * 4], 255);
        assert!(out.rgba.as_chunks::<4>().0.iter().map(|p| p[0]).is_sorted(), "a ramp");
    }

    #[test]
    fn the_luma_reference_keeps_the_originals_colour_on_average_and_the_shading_where_it_can() {
        // Half the pixels at 200 and half at 100: a reference of 106 keeps 97 percent
        let mut rgba = Vec::new();
        for level in [200_u8, 100] {
            for _ in 0..50 {
                rgba.extend_from_slice(&[level, level, level, 255]);
            }
        }
        let image = Image::new(100, 1, rgba);
        assert!((105..=107).contains(&image.luma_ref), "{}", image.luma_ref);
        // A flat image's reference is a little above its level (the 97 percent); transparent
        // pixels count for nothing
        let mut flat = Vec::new();
        for _ in 0..10 {
            flat.extend_from_slice(&[80, 80, 80, 255]);
            flat.extend_from_slice(&[255, 255, 255, 0]);
        }
        assert_eq!(Image::new(20, 1, flat).luma_ref, 82);
        assert_eq!(Image::new(1, 1, vec![9, 9, 9, 0]).luma_ref, 255);
    }
}
