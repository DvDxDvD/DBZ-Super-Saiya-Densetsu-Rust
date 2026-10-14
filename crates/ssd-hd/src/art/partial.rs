//! Finding the pose a partial pose is part of. A fighter fades in or out as a pattern of stripes
//! (some of its pixels are left out), and a sprite the screen's edge cuts has pieces missing: the
//! pixels that remain are a subset of a pose the art index holds. The pose is looked for among the
//! fighter's own (the poses made for the palette the pieces are drawn with), the smallest first,
//! at every place its box holds the partial one's: the first whose silhouette has every opaque
//! pixel of the partial pose is the one.

use super::poses::Pose;
use crate::art_index::{ArtIndex, Palette, PaletteGroup};

/// The fewest opaque pixels a partial pose looks for its pose with: fewer are in too many.
pub(crate) const MIN_OPAQUE: u32 = 48;

/// How far (summed channel differences) the colours of a palette group may be from the palette
/// the pieces are drawn with, for the group to be the fighter's.
const MAX_PALETTE_DISTANCE: u32 = 24;

/// How far (summed channel differences) two colours of one index may be and still be the same.
const SAME_COLOUR: u32 = 24;

/// The object of `index` that holds `pose`, and where the pose's box sits in the object's: the
/// object's index, and the column and row of the object's box the pose's top-left pixel falls on.
///
/// The groups of palettes with colours are tried best first: the groups close enough to the colours
/// the pose is drawn with, then the others, those that have the most of the pose's colours right
/// first (a pose drawn in other colours than any art was made for: a fade-in whitened or darkened
/// by the screen's colour math, a flash on some indexes, a body in another fighter's colours). Among
/// the groups on a par the smallest silhouette that holds every opaque pixel of the pose is the one.
pub(crate) fn containing(
    index: &ArtIndex,
    row: &[[u8; 3]; 16],
    pose: &Pose,
) -> Option<(u32, i32, i32)> {
    let usable = |at: &u32| index.objects()[*at as usize].field() == pose.field;
    let points: Vec<(usize, usize)> = (0..pose.h)
        .flat_map(|y| (0..pose.w).map(move |x| (x, y)))
        .filter(|&(x, y)| pose.arr[y * pose.w + x] != 0)
        .collect();
    let opaque = points.len() as u32;
    let mut used = 0_u16;
    for &(x, y) in &points {
        used |= 1 << (pose.arr[y * pose.w + x] & 15);
    }
    // A group of no colour (the effects' art) is never a fighter's
    let mut groups: Vec<(bool, std::cmp::Reverse<u32>, u32, &PaletteGroup)> = index
        .palette_groups()
        .iter()
        .filter(|group| group.palette.mask != 0 && group.objects.iter().any(usable))
        .map(|group| {
            let distance = group.palette.distance(row);
            let matching = matching(&group.palette, row, used);
            (distance > MAX_PALETTE_DISTANCE, std::cmp::Reverse(matching), distance, group)
        })
        .collect();
    groups.sort_by_key(|&(far, matching, distance, _)| (far, matching, distance));
    // Each run of groups that are on a par: the smallest silhouette that holds the pose
    let mut start = 0;
    while start < groups.len() {
        let par = (groups[start].0, groups[start].1);
        let end = start + groups[start..].iter().take_while(|g| (g.0, g.1) == par).count();
        let mut best: Option<(u32, u32, i32, i32)> = None;
        for &(.., group) in &groups[start..end] {
            for &at in group.objects.iter().filter(|at| usable(at)) {
                let object = &index.objects()[at as usize];
                let (w, h) = (usize::from(object.width), usize::from(object.height));
                let size = object.opaque();
                if size < opaque || w < pose.w || h < pose.h || best.is_some_and(|b| size >= b.0) {
                    continue;
                }
                'places: for dy in 0..=h - pose.h {
                    for dx in 0..=w - pose.w {
                        if points.iter().all(|&(x, y)| object.opaque_at(x + dx, y + dy)) {
                            best = Some((size, at, dx as i32, dy as i32));
                            break 'places;
                        }
                    }
                }
            }
        }
        if let Some((_, at, dx, dy)) = best {
            return Some((at, dx, dy));
        }
        start = end;
    }
    None
}

/// How many of the colour indexes in `used` (a bit each) that `palette` has colours for the pose
/// draws in the colour the palette has (to within [`SAME_COLOUR`], summed over the channels).
fn matching(palette: &Palette, row: &[[u8; 3]; 16], used: u16) -> u32 {
    (1..16)
        .filter(|&i| used >> i & 1 != 0 && palette.mask >> i & 1 != 0)
        .filter(|&i| {
            let colour = palette.colors[i];
            colour.iter().zip(&row[i]).map(|(a, b)| u32::from(a.abs_diff(*b))).sum::<u32>()
                <= SAME_COLOUR
        })
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art_index::{Object, Palette};

    type Points = Vec<(usize, usize)>;

    fn block(width: usize, height: usize) -> Points {
        (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).collect()
    }

    fn object(id: &str, width: u16, height: u16, opaque: &[(usize, usize)]) -> Object {
        let mut mask = vec![0_u8; (usize::from(width) * usize::from(height)).div_ceil(8)];
        for &(x, y) in opaque {
            let at = y * usize::from(width) + x;
            mask[at / 8] |= 1 << (at % 8);
        }
        let mut palette = Palette { mask: 0b10, ..Palette::default() };
        palette.colors[1] = [200, 0, 0];
        Object {
            key: 0,
            id: id.to_owned(),
            luma: false,
            palette,
            mask,
            width,
            height,
            art: String::new(),
        }
    }

    fn pose(w: usize, h: usize, opaque: &[(usize, usize)]) -> Pose {
        let mut arr = vec![0_u8; w * h];
        for &(x, y) in opaque {
            arr[y * w + x] = 1;
        }
        Pose {
            key: 1,
            fighter: true,
            field: false,
            cut: false,
            sprites: 1,
            first: 0,
            priority: 2,
            palette: 1,
            w,
            h,
            arr,
            rgba: Vec::new(),
            org: (50, 60),
            shadow: false,
            excluded: 0,
            recolour: None,
            bg: false,
        }
    }

    fn row() -> [[u8; 3]; 16] {
        let mut row = [[0_u8; 3]; 16];
        row[1] = [200, 0, 0];
        row
    }

    #[test]
    fn a_pose_with_pixels_missing_is_found_in_the_smallest_pose_that_holds_it() {
        let big = object("big", 10, 10, &block(10, 10));
        let small = object("small", 8, 8, &block(8, 8));
        let index = ArtIndex::new(Vec::new(), vec![big, small]);
        // Every other column of the 8 x 8 square
        let stripes: Points = block(8, 8).into_iter().filter(|&(x, _)| x % 2 == 0).collect();
        assert_eq!(containing(&index, &row(), &pose(8, 8, &stripes)), Some((1, 0, 0)));
        // A piece of it, at the lower right: the same pose
        let corner: Points = block(3, 3).into_iter().map(|(x, y)| (x + 5, y + 5)).collect();
        assert_eq!(containing(&index, &row(), &pose(8, 8, &corner)), Some((1, 0, 0)));
        // A smaller box inside the big one's: placed where its pixels fall
        let inner: Points = block(3, 3).into_iter().collect();
        let found = containing(&index, &row(), &pose(3, 3, &inner)).unwrap();
        assert_eq!(found, (1, 0, 0), "the smallest pose that holds it");
    }

    #[test]
    fn a_pose_that_no_silhouette_holds_is_not_placed() {
        let index = ArtIndex::new(Vec::new(), vec![object("a", 4, 4, &block(4, 4))]);
        let wider = pose(5, 4, &[(4, 0), (0, 0)]);
        assert_eq!(containing(&index, &row(), &wider), None, "it does not fit the box");
    }

    /// An object whose art was made for colour 1 being `colour`.
    fn coloured(id: &str, width: u16, height: u16, colour: [u8; 3]) -> Object {
        let mut found = object(id, width, height, &block(usize::from(width), usize::from(height)));
        found.palette.colors[1] = colour;
        found
    }

    #[test]
    fn a_pose_in_other_colours_is_found_by_its_silhouette_in_the_closest_palette() {
        // A fade-in whitened by the colour math: no palette is its own, the silhouette decides, the
        // smallest one that holds the pose first
        let index = ArtIndex::new(
            Vec::new(),
            vec![
                coloured("red-big", 10, 10, [200, 0, 0]),
                coloured("blue-small", 8, 8, [0, 0, 200]),
            ],
        );
        let mut white = row();
        white[1] = [255, 255, 255];
        assert_eq!(containing(&index, &white, &pose(8, 8, &[(0, 0), (2, 2)])), Some((1, 0, 0)));
        // A palette of the right colours wins over a smaller silhouette of the wrong ones
        let mut blue = row();
        blue[1] = [0, 0, 200];
        assert_eq!(containing(&index, &blue, &pose(8, 8, &[(0, 0)])), Some((1, 0, 0)));
        let (big_blue, small_red) =
            (coloured("blue-big", 10, 10, [0, 0, 200]), coloured("red-small", 8, 8, [200, 0, 0]));
        let index = ArtIndex::new(Vec::new(), vec![big_blue, small_red]);
        assert_eq!(containing(&index, &blue, &pose(8, 8, &[(0, 0)])), Some((0, 0, 0)));
    }

    #[test]
    fn art_of_no_colour_is_never_a_fighters() {
        let mut effect = object("effect", 8, 8, &block(8, 8));
        effect.palette = Palette::default();
        let index = ArtIndex::new(Vec::new(), vec![effect]);
        assert_eq!(containing(&index, &row(), &pose(4, 4, &[(0, 0)])), None);
    }

    #[test]
    fn a_silhouette_the_poses_pixels_leave_out_does_not_hold_it() {
        let ring: Points = block(4, 4).into_iter().filter(|&p| p != (1, 1)).collect();
        let index = ArtIndex::new(Vec::new(), vec![object("ring", 4, 4, &ring)]);
        assert_eq!(containing(&index, &row(), &pose(4, 4, &[(1, 1)])), None);
        assert_eq!(containing(&index, &row(), &pose(4, 4, &[(0, 0), (2, 2)])), Some((0, 0, 0)));
    }
}
