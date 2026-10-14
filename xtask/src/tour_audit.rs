//! The art audit of `hd-tour` (`--audit DIR`): for every fighter of a battle frame, where its art
//! came from (the art made for its pixels, the art of its mirror image, a pose of the same shape in
//! other colours, a piece of a bigger pose, or none), so a pose drawn from borrowed or missing art
//! is found by what it shows and not hidden in a coverage percentage. A part writes
//! `DIR/<part>.json`: the distinct fighters seen with their source and art, and the frames that
//! show one of the faults:
//!
//! * `borrowed`: a whole pose drawn from a piece of another pose's art;
//! * `none`: a whole fighter with no art;
//! * `mixed`: a frame where fighters of one palette (the units of a formation) are drawn, some from
//!   art and some with none;
//! * `split`: a frame where two fighters of the same pixels are drawn from different art.
//!
//! The files are derived from the game and never tracked.

use serde::Serialize;
use ssd_hd::pack::Pack;
use ssd_hd::{ArtSource, PoseStat};
use std::collections::BTreeMap;
use std::path::Path;

/// The fewest opaque pixels of a pose the audit looks at (the capture's 40 and a margin: smaller
/// poses are sparks and pieces).
const MIN_OPAQUE: u32 = 100;
/// The share (per mille) of lonely pixels from which a pose is a fade-in's stripes, not whole.
const STRIPY: u32 = 200;
/// The examples kept of each fault.
const EXAMPLES: usize = 12;

#[derive(Debug, Serialize)]
struct Row {
    key: String,
    palette: u8,
    source: &'static str,
    art: String,
    width: usize,
    height: usize,
    opaque: u32,
    frames: u64,
    first_part: String,
    first_frame: u64,
}

#[derive(Debug, Default, Serialize)]
struct Fault {
    frames: u64,
    examples: Vec<String>,
}

/// What a part's frames showed of the fighters' art.
#[derive(Debug, Default)]
pub struct Audit {
    rows: BTreeMap<(u64, u8, u8, String), Row>,
    faults: BTreeMap<&'static str, Fault>,
}

fn name(source: ArtSource) -> &'static str {
    match source {
        ArtSource::None => "none",
        ArtSource::Exact => "exact",
        ArtSource::Mirror => "mirror",
        ArtSource::Shaped => "shaped",
        ArtSource::Held => "held",
    }
}

impl Audit {
    fn fault(&mut self, kind: &'static str, part: &str, frame: u64, what: String) {
        let fault = self.faults.entry(kind).or_default();
        fault.frames += 1;
        if fault.examples.len() < EXAMPLES {
            fault.examples.push(format!("{part} f{frame}: {what}"));
        }
    }

    /// Notes the fighters of a frame of a battle (`uses` are the poses the renderer found).
    pub fn note(&mut self, uses: &[PoseStat], pack: &Pack, part: &str, frame: u64) {
        let objects = pack.art.objects();
        let id = |pose: &PoseStat| -> String {
            pose.art
                .and_then(|at| objects.get(at as usize))
                .map(|object| object.id.clone())
                .unwrap_or_default()
        };
        let whole: Vec<&PoseStat> = uses
            .iter()
            .filter(|p| p.fighter && !p.field && p.opaque >= MIN_OPAQUE && p.stripy < STRIPY)
            .collect();
        for pose in &whole {
            let art = id(pose);
            let row = self
                .rows
                .entry((pose.key, pose.palette, pose.source as u8, art.clone()))
                .or_insert_with(|| Row {
                    key: format!("{:016x}", pose.key),
                    palette: pose.palette,
                    source: name(pose.source),
                    art,
                    width: pose.size.0,
                    height: pose.size.1,
                    opaque: pose.opaque,
                    frames: 0,
                    first_part: part.to_owned(),
                    first_frame: frame,
                });
            row.frames += 1;
        }
        for pose in &whole {
            match pose.source {
                ArtSource::Held => {
                    let what = format!("{:016x} from {}", pose.key, id(pose));
                    self.fault("borrowed", part, frame, what);
                }
                ArtSource::None => self.fault("none", part, frame, format!("{:016x}", pose.key)),
                _ => {}
            }
        }
        // A formation: the fighters of one palette, some drawn from art and some not
        let mut by_palette: BTreeMap<u8, (u32, u32)> = BTreeMap::new();
        for pose in &whole {
            let held = by_palette.entry(pose.palette).or_default();
            if pose.source == ArtSource::None {
                held.1 += 1;
            } else {
                held.0 += 1;
            }
        }
        for (palette, (drawn, bare)) in by_palette {
            if drawn > 0 && bare > 0 {
                let what = format!("palette {palette}: {drawn} from art, {bare} with none");
                self.fault("mixed", part, frame, what);
            }
        }
        // The same pixels drawn from different art
        let mut by_key: BTreeMap<u64, Vec<(ArtSource, String)>> = BTreeMap::new();
        for pose in &whole {
            by_key.entry(pose.key).or_default().push((pose.source, id(pose)));
        }
        for (key, seen) in by_key {
            if seen.windows(2).any(|pair| pair[0] != pair[1]) {
                self.fault("split", part, frame, format!("{key:016x}: {seen:?}"));
            }
        }
    }

    /// Writes the part's file under `dir` (nothing when no fighter was seen).
    pub fn write(&self, dir: &Path, part: &str) {
        if self.rows.is_empty() && self.faults.is_empty() {
            return;
        }
        let _ = std::fs::create_dir_all(dir);
        let rows: Vec<&Row> = self.rows.values().collect();
        let json = serde_json::json!({ "part": part, "rows": rows, "faults": self.faults });
        let _ = std::fs::write(dir.join(format!("{part}.json")), json.to_string());
    }
}
