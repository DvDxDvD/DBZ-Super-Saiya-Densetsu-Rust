//! The mixed-figure check of `hd-art scenes` and `hd-art figures`: a field figure is drawn from one
//! art or from the original's pixels, never from both. A figure whose pieces are partly drawn from
//! art and partly left as the original draws them (an arm the art does not cover, a piece in another
//! palette, a part the grouping cut off) shows half HD and half pixels, or from two arts that do not
//! agree at the seam.

use crate::scenes::{Reason, Verdict};
use ssd_hd::{ArtSource, PoseStat};

/// How far (pixels) the box of a piece may be from the box of an art-drawn figure and still be
/// read as part of it: the game's own rule for the pieces of one group.
const TOUCH: i32 = 2;

/// The fewest pixels a bare piece shows for the check to look at it (a spark of two pixels is not
/// a part of a figure).
const MIN_VISIBLE: u32 = 6;

/// The gap between the boxes of two poses (0 when they touch or overlap).
fn gap(a: &PoseStat, b: &PoseStat) -> i32 {
    let dx = (a.org.0 - (b.org.0 + b.size.0 as i32)).max(b.org.0 - (a.org.0 + a.size.0 as i32));
    let dy = (a.org.1 - (b.org.1 + b.size.1 as i32)).max(b.org.1 - (a.org.1 + a.size.1 as i32));
    dx.max(dy).max(0)
}

/// Whether the boxes of `a` and `b` share at least a quarter of the smaller one's area.
fn overlap(a: &PoseStat, b: &PoseStat) -> bool {
    let w = (a.org.0 + a.size.0 as i32).min(b.org.0 + b.size.0 as i32) - a.org.0.max(b.org.0);
    let h = (a.org.1 + a.size.1 as i32).min(b.org.1 + b.size.1 as i32) - a.org.1.max(b.org.1);
    if w <= 0 || h <= 0 {
        return false;
    }
    let smaller = (a.size.0 * a.size.1).min(b.size.0 * b.size.1) as i32;
    w * h * 4 >= smaller
}

/// One figure that is drawn from art and from something else in a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mixed {
    /// The art-drawn figure: its pose key, its art and its palette.
    pub figure: (u64, u32, u8),
    /// What goes with it: the key of a piece drawn from the original's pixels, or (with
    /// `other_art`) of a second art-drawn figure over the first.
    pub other: u64,
    pub other_art: Option<u32>,
    pub palette: u8,
    pub visible: u32,
}

impl Mixed {
    /// The name of the case in reports: the art's ids, so one case is one name over its frames (a
    /// piece with no art is named by its palette).
    #[must_use]
    pub fn name(&self, art_id: &dyn Fn(u32) -> String) -> String {
        match self.other_art {
            None => format!(
                "{} (palette {}) with original pixels (palette {})",
                art_id(self.figure.1),
                self.figure.2,
                self.palette
            ),
            Some(other) => format!(
                "{} over {} (palette {})",
                art_id(self.figure.1),
                art_id(other),
                self.palette
            ),
        }
    }
}

/// The figures of `poses` (a frame outside a battle) that are drawn partly from art and partly
/// from the original's pixels, or from two arts that overlap.
#[must_use]
pub fn find(poses: &[PoseStat]) -> Vec<Mixed> {
    let drawn = || poses.iter().filter(|p| p.matched && p.field && !p.shadow && p.art.is_some());
    // The sprites some art draws: a pose with no art is bare only where no art took its sprites
    let taken = poses.iter().filter(|p| p.matched).fold(0_u128, |all, p| all | p.sprites);
    let mut found = Vec::new();
    for figure in drawn() {
        let Some(art) = figure.art else { continue };
        for piece in poses {
            if std::ptr::eq(piece, figure) || piece.shadow || piece.matched || !piece.field {
                continue;
            }
            if piece.sprites & !taken == 0 || piece.visible < MIN_VISIBLE {
                continue;
            }
            if piece.palette == figure.palette && gap(piece, figure) <= TOUCH {
                found.push(Mixed {
                    figure: (figure.key, art, figure.palette),
                    other: piece.key,
                    other_art: None,
                    palette: piece.palette,
                    visible: piece.visible,
                });
            }
        }
        for other in drawn() {
            let Some(other_art) = other.art else { continue };
            if figure.key < other.key
                && figure.palette == other.palette
                && figure.art != other.art
                && overlap(figure, other)
            {
                found.push(Mixed {
                    figure: (figure.key, art, figure.palette),
                    other: other.key,
                    other_art: Some(other_art),
                    palette: other.palette,
                    visible: other.visible,
                });
            }
        }
    }
    found
}

/// A figure drawn from art whose box is not the original pose's: the art was fitted to a pose of
/// another size or shape, so the figure looks stretched. The art of a pose is made at a whole
/// scale of the pose's box (4x); a width and a height that disagree on the scale by more than
/// [`STRETCH`] pixels of the original are a stretch. A piece of a bigger art (a cut sprite) is
/// not looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stretched {
    pub art: u32,
    pub pose: (usize, usize),
    pub drawn: (u16, u16),
}

/// How many pixels of the original a figure may be off its art's aspect.
const STRETCH: f32 = 2.0;

/// The figures of `poses` drawn from an art whose box does not fit their own, `art_size` giving the
/// size of an art object.
#[must_use]
pub fn find_stretched(
    poses: &[PoseStat],
    art_size: &dyn Fn(u32) -> Option<(u16, u16)>,
) -> Vec<Stretched> {
    poses
        .iter()
        .filter(|p| p.matched && p.field && !p.shadow && p.source != ArtSource::Held)
        .filter_map(|p| {
            let art = p.art?;
            let drawn = art_size(art)?;
            let (w, h) = (p.size.0 as f32, p.size.1 as f32);
            let (sx, sy) = (f32::from(drawn.0) / w, f32::from(drawn.1) / h);
            let off = (sx - sy).abs() * w.max(h) / sx.max(sy);
            (off > STRETCH).then_some(Stretched { art, pose: p.size, drawn })
        })
        .collect()
}

/// The names of the faults of a field frame, as [`Mixed::name`] gives them, and the stretched
/// figures'.
#[must_use]
pub fn names(
    poses: &[PoseStat],
    art_id: &dyn Fn(u32) -> String,
    art_size: &dyn Fn(u32) -> Option<(u16, u16)>,
) -> Vec<String> {
    let mut out: Vec<String> = find(poses).iter().map(|c| c.name(art_id)).collect();
    out.extend(find_stretched(poses, art_size).iter().map(|s| {
        format!(
            "{} stretched (art {}x{} on a pose of {}x{})",
            art_id(s.art),
            s.drawn.0,
            s.drawn.1,
            s.pose.0,
            s.pose.1
        )
    }));
    out
}

/// Adds a [`Reason::Mixed`] to `verdict` for each mixed figure of the frame, and a
/// [`Reason::Stretched`] for each figure drawn from art of another box.
pub fn check(
    poses: &[PoseStat],
    art_id: &dyn Fn(u32) -> String,
    art_size: &dyn Fn(u32) -> Option<(u16, u16)>,
    verdict: &mut Verdict,
) {
    for case in find(poses) {
        verdict.add(Reason::Mixed, case.name(art_id), u64::from(case.visible));
    }
    for case in find_stretched(poses, art_size) {
        let name = format!(
            "{} stretched (art {}x{} on a pose of {}x{})",
            art_id(case.art),
            case.drawn.0,
            case.drawn.1,
            case.pose.0,
            case.pose.1
        );
        verdict.add(Reason::Stretched, name, (case.pose.0 * case.pose.1) as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(
        key: u64,
        art: Option<u32>,
        palette: u8,
        org: (i32, i32),
        size: (usize, usize),
        sprites: u128,
    ) -> PoseStat {
        PoseStat {
            fighter: false,
            field: true,
            shadow: false,
            sprites,
            excluded: 0,
            picture: false,
            matched: art.is_some(),
            recoloured: false,
            visible: 100,
            opaque: 100,
            stripy: 0,
            key,
            palette,
            source: if art.is_some() { ArtSource::Exact } else { ArtSource::None },
            art,
            org,
            size,
            look: 0,
        }
    }

    #[test]
    fn a_figure_drawn_from_art_with_a_bare_piece_touching_it_is_mixed() {
        let figure = pose(1, Some(5), 4, (40, 40), (20, 30), 0b0111);
        let arm = pose(2, None, 4, (58, 50), (6, 10), 0b1000);
        assert_eq!(find(&[figure, arm]).len(), 1);
    }

    #[test]
    fn a_bare_piece_far_from_the_figure_is_another_thing() {
        let figure = pose(1, Some(5), 4, (40, 40), (20, 30), 0b0111);
        let npc = pose(2, None, 4, (90, 50), (16, 24), 0b1000);
        assert_eq!(find(&[figure, npc]).len(), 0);
    }

    #[test]
    fn a_piece_whose_sprites_the_art_took_is_not_bare() {
        // an effect's cluster over the same sprites the figure's art draws
        let figure = pose(1, Some(5), 4, (40, 40), (20, 30), 0b0111);
        let mut cluster = pose(2, None, 4, (42, 42), (10, 10), 0b0011);
        cluster.field = false;
        assert_eq!(find(&[figure, cluster]).len(), 0);
    }

    #[test]
    fn two_arts_over_one_another_in_one_palette_are_mixed() {
        let a = pose(1, Some(5), 4, (40, 40), (20, 30), 0b0011);
        let b = pose(2, Some(6), 4, (44, 44), (20, 30), 0b1100);
        assert_eq!(find(&[a.clone(), b]).len(), 1);
        // the same art twice (two soldiers of one kind) or another palette is not
        let same = pose(2, Some(5), 4, (44, 44), (20, 30), 0b1100);
        assert_eq!(find(&[a.clone(), same]).len(), 0);
        let other = pose(2, Some(6), 3, (44, 44), (20, 30), 0b1100);
        assert_eq!(find(&[a, other]).len(), 0);
    }

    #[test]
    fn a_bare_piece_of_another_palette_is_another_character() {
        let figure = pose(1, Some(5), 4, (40, 40), (20, 30), 0b0111);
        let other = pose(2, None, 7, (58, 50), (6, 10), 0b1000);
        assert_eq!(find(&[figure, other]).len(), 0);
    }

    #[test]
    fn art_of_another_aspect_than_its_pose_is_stretched() {
        let figure = pose(1, Some(5), 4, (40, 40), (15, 23), 0b0111);
        let size = |at: u32| if at == 5 { Some((60, 92)) } else { Some((60, 120)) };
        assert_eq!(find_stretched(std::slice::from_ref(&figure), &size).len(), 0);
        let long = pose(1, Some(6), 4, (40, 40), (15, 23), 0b0111);
        assert_eq!(find_stretched(&[long], &size).len(), 1);
    }

    #[test]
    fn a_ground_shadow_is_not_a_piece() {
        let figure = pose(1, Some(5), 4, (40, 40), (20, 30), 0b0111);
        let mut shadow = pose(2, None, 4, (44, 66), (14, 6), 0b1000);
        shadow.shadow = true;
        assert_eq!(find(&[figure, shadow]).len(), 0);
    }
}
