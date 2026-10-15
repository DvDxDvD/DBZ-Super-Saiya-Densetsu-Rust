//! The flicker check of `hd-art scenes`: a character whose original pixels do not change must keep
//! the art it is drawn from. The game rotates the order of its sprites and a group of sprites can
//! be cut differently from one frame to the next, so a pose can be found as one thing in a frame
//! and as another in the next; the pictures then alternate between two drawings of one standing
//! figure.

use crate::scenes::{Reason, Verdict};
use ssd_hd::PoseStat;
use std::collections::VecDeque;

/// Frames remembered.
const MEMORY: usize = 4;

/// What a pose was drawn from, for comparing two frames.
fn art_of(pose: &PoseStat) -> (bool, Option<u32>) {
    (pose.matched, pose.art)
}

/// Whether two poses are the same figure: the same palette and boxes that overlap by at least
/// 60 percent of the smaller one.
fn same_place(a: &PoseStat, b: &PoseStat) -> bool {
    if a.palette != b.palette {
        return false;
    }
    let (ax1, ay1) = (a.org.0 + a.size.0 as i32, a.org.1 + a.size.1 as i32);
    let (bx1, by1) = (b.org.0 + b.size.0 as i32, b.org.1 + b.size.1 as i32);
    let w = ax1.min(bx1) - a.org.0.max(b.org.0);
    let h = ay1.min(by1) - a.org.1.max(b.org.1);
    if w <= 0 || h <= 0 {
        return false;
    }
    let smaller = (a.size.0 * a.size.1).min(b.size.0 * b.size.1) as i32;
    w * h * 10 >= smaller * 6
}

/// Whether `figure` lies inside the box of an effect of `frame` that was drawn from art: the
/// sprites of the figure were read as a part of the effect's cluster (the effect's sprites touch
/// them) and the cluster's own art, made with the figure in it, drew them.
fn under_effect(figure: &PoseStat, frame: &[PoseStat]) -> bool {
    frame.iter().any(|q| {
        let (qx1, qy1) = (q.org.0 + q.size.0 as i32, q.org.1 + q.size.1 as i32);
        q.matched
            && !q.field
            && !q.fighter
            && q.org.0 <= figure.org.0
            && q.org.1 <= figure.org.1
            && qx1 >= figure.org.0 + figure.size.0 as i32
            && qy1 >= figure.org.1 + figure.size.1 as i32
    })
}

/// Remembers what the field poses of the frames before were drawn from.
#[derive(Default)]
pub struct Tracker {
    history: VecDeque<Vec<PoseStat>>,
}

impl Tracker {
    /// The frames before are not the ones that come (the machine jumped, or frames were skipped).
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Looks the poses of the frame up against the frames before; adds a [`Reason::Flicker`] to
    /// `verdict` for each figure whose original pixels are the same in two consecutive frames (or
    /// that was one thing and is another again) while its art is not.
    pub fn check(&mut self, poses: &[PoseStat], verdict: &mut Verdict) {
        let poses: Vec<PoseStat> = poses.iter().filter(|p| p.visible >= 40).cloned().collect();
        if self.history.len() >= 2 {
            let (older, before) =
                (&self.history[self.history.len() - 2], &self.history[self.history.len() - 1]);
            for pose in poses.iter().filter(|p| p.field) {
                // (the same kind of pose: a field group is not compared with an effect's cluster)
                let like = |q: &&PoseStat| q.field == pose.field && same_place(pose, q);
                let Some(mid) = before.iter().find(like) else { continue };
                let Some(old) = older.iter().find(like) else { continue };
                // The figure was drawn from some art, from another in the frame between, and from the
                // first again: a switch back and forth (a change of art once, a palette's variants
                // as a fade goes by, is not a flicker)
                if art_of(old) != art_of(pose) || art_of(mid) == art_of(pose) {
                    continue;
                }
                // The original put something over the figure in one of the frames (a flash, a
                // shadow that touches it) and the art made for the whole cluster was drawn: the
                // change is the original's own
                let covered = |figure: &PoseStat, frame: &[PoseStat]| {
                    !figure.matched && under_effect(figure, frame)
                };
                if covered(mid, before) || covered(old, older) || covered(pose, &poses) {
                    continue;
                }
                let same_box = mid.org == pose.org && mid.size == pose.size;
                let what = if same_box {
                    // the original's pixels in the box must be the same in all three frames
                    if mid.look != pose.look || old.look != pose.look {
                        continue;
                    }
                    "same pixels, other art in between"
                } else {
                    "alternates"
                };
                verdict.add(
                    Reason::Flicker,
                    format!("{:016x} / {:016x}: {what}", mid.key, pose.key),
                    u64::from(pose.visible),
                );
            }
        }
        self.history.push_back(poses);
        while self.history.len() > MEMORY {
            self.history.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_hd::ArtSource;

    fn stat(
        key: u64,
        field: bool,
        art: Option<u32>,
        org: (i32, i32),
        size: (usize, usize),
    ) -> PoseStat {
        PoseStat {
            fighter: false,
            field,
            matched: art.is_some(),
            recoloured: false,
            visible: 300,
            opaque: 300,
            stripy: 0,
            key,
            palette: 0,
            source: if art.is_some() { ArtSource::Exact } else { ArtSource::None },
            art,
            org,
            size,
            look: 7,
        }
    }

    /// Runs three frames through the tracker and returns the number of faults of the last.
    fn faults(frames: [Vec<PoseStat>; 3]) -> usize {
        let mut tracker = Tracker::default();
        let mut found = 0;
        for poses in frames {
            let mut verdict = Verdict::default();
            tracker.check(&poses, &mut verdict);
            found = verdict.items.len();
        }
        found
    }

    #[test]
    fn a_figure_that_goes_without_art_for_one_frame_is_a_flicker() {
        let figure = |art| stat(1, true, art, (40, 40), (24, 21));
        assert_eq!(faults([vec![figure(Some(1))], vec![figure(None)], vec![figure(Some(1))]]), 1);
    }

    #[test]
    fn a_figure_an_effect_with_art_covers_for_one_frame_is_the_originals_own_change() {
        let figure = |art| stat(1, true, art, (40, 40), (24, 21));
        // a shadow touches the figure for one frame: the cluster's art draws both
        let cluster = stat(2, false, Some(9), (36, 38), (41, 24));
        assert_eq!(
            faults([vec![figure(Some(1))], vec![cluster, figure(None)], vec![figure(Some(1))]]),
            0
        );
    }

    #[test]
    fn the_effect_may_cover_the_figure_in_the_first_or_the_last_frame_too() {
        let figure = |art| stat(1, true, art, (40, 40), (24, 21));
        let cluster = stat(2, false, Some(9), (36, 38), (41, 24));
        assert_eq!(
            faults([
                vec![cluster.clone(), figure(None)],
                vec![figure(Some(1))],
                vec![cluster, figure(None)]
            ]),
            0
        );
    }

    #[test]
    fn an_effect_that_does_not_cover_the_figure_does_not_excuse_it() {
        let figure = |art| stat(1, true, art, (40, 40), (24, 21));
        let far = stat(2, false, Some(9), (100, 100), (20, 20));
        assert_eq!(
            faults([vec![figure(Some(1))], vec![far, figure(None)], vec![figure(Some(1))]]),
            1
        );
    }
}
