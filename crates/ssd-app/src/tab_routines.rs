//! The Routines tab of the debug panel: the share of the instructions that run as Rust, the bars
//! of the last 60 frames, and one row per routine with its mode (O, R or V, applied live), calls,
//! fallbacks and divergences. The figures are plain code, tested without a window; the drawing at
//! the end only reads them and pushes [`PanelAction::SetRoutineMode`].

use crate::hook_setup::HooksArg;
use crate::panel::{Controls, HooksSwitch, PanelAction, grouped};
use crate::theme::{
    self, ACCENT, BAR, BRIGHT, CONTROL, CONTROL_HOVER, DISABLED, DIVERGE, DIVERGE_FILL, LINE,
    LINE_STRONG, MUTED, MUTED_LIGHT, ORIGINAL_FILL, RUST_FILL, TEXT, TEXT_STRONG, VERIFY,
    VERIFY_FILL,
};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Layout, Margin, Rect,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use ssd_oracle::{Coverage, FrameStats, HookMode, Machine, Symbols};
use std::collections::VecDeque;

/// Frames in the bar strip, and in the figures of the header and the summary.
pub const HISTORY: usize = 60;
/// The two RNGs run from the idle wait between frames: "without idle RNG" leaves them out.
pub const IDLE_RNG: [&str; 2] = ["rng_next_a", "rng_next_b"];
/// Fallbacks above this share of the calls, in percent, are shown in orange.
pub const FALLBACK_ALERT: f64 = 15.0;
/// Shown in place of a Rust share that cannot be known yet (see [`means_missing`]).
pub const MEANS_HINT: &str = "needs a Verify run for the means";

/// One frame of the window: what the machine counted for it, and the Rust estimate of the idle RNG
/// routines in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sample {
    stats: FrameStats,
    idle_rng: u64,
}

/// The last frames the machine rendered, oldest first. The bar strip, the header and the summary
/// are all figures of this window.
#[derive(Debug, Default)]
pub struct FrameHistory {
    frames: VecDeque<Sample>,
    /// The idle RNG's Rust estimate at the last frame noted, or at the last [`FrameHistory::rebase`]:
    /// a frame's share is the growth since then.
    rng_base: u64,
}

impl FrameHistory {
    /// Notes the statistics of the frame the machine rendered last (`None` before its first one),
    /// with `idle_rng_total`, the idle RNG's Rust estimate as the machine has it now (see
    /// [`idle_rng_total`]). A frame already noted is not noted again: the loop may ask more than
    /// once for one frame.
    pub fn record(&mut self, stats: Option<FrameStats>, idle_rng_total: u64) {
        let Some(stats) = stats else { return };
        if self.frames.back().is_some_and(|last| last.stats.frame == stats.frame) {
            return;
        }
        // A total below the base means the counters started again (a table was installed without a
        // rebase): they count from zero, and so does this frame's share.
        let idle_rng = if idle_rng_total >= self.rng_base {
            idle_rng_total - self.rng_base
        } else {
            idle_rng_total
        };
        self.rng_base = idle_rng_total;
        if self.frames.len() == HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back(Sample { stats, idle_rng });
    }

    /// Restarts the idle RNG's share at `idle_rng_total`. The machine restarts its own per-frame
    /// figures when its table or its means change, so the frame after the change is not compared
    /// with the ones before it.
    pub fn rebase(&mut self, idle_rng_total: u64) {
        self.rng_base = idle_rng_total;
    }

    /// Whether no frame has been noted yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The share of Rust instructions of each of the last [`HISTORY`] frames, in percent, oldest
    /// first; `None` for the frames before the first one noted.
    #[must_use]
    pub fn bars(&self) -> Vec<Option<f64>> {
        let missing = HISTORY - self.frames.len();
        std::iter::repeat_n(None, missing)
            .chain(self.frames.iter().map(|sample| Some(sample.stats.coverage_percent())))
            .collect()
    }

    /// The instructions of the window: the ones the machine executed and the Rust estimate.
    #[must_use]
    pub fn coverage(&self) -> Coverage {
        self.frames.iter().fold(Coverage::default(), |sum, sample| Coverage {
            original_instructions: sum.original_instructions + sample.stats.original_instructions,
            rust_instructions_estimated: sum.rust_instructions_estimated
                + sample.stats.rust_instructions_estimated,
        })
    }

    /// The Rust estimate of the idle RNG routines over the window.
    #[must_use]
    pub fn idle_rng(&self) -> u64 {
        self.frames.iter().map(|sample| sample.idle_rng).sum()
    }
}

/// The Rust estimate of the idle RNG routines of `machine`, from its counters as they are now: the
/// sum of their [`HookStats::estimated_rust_instructions`].
///
/// The table of a recompiled build has tens of thousands of routines and this is asked for every
/// frame the machine runs, so it looks at the two routines' counters and nothing else: no copy of
/// the table's counters, no scan of its names but the first time (and when the table changes).
#[must_use]
pub fn idle_rng_total(machine: &Machine) -> u64 {
    let Some(table) = machine.hook_table() else { return 0 };
    IDLE_RNG_KEYS.with(|cache| {
        let mut cache = cache.borrow_mut();
        // The keys are the ones of this table if it has as many routines as the table they were
        // found in and each still carries the name it was found under.
        let valid = cache.len == table.len()
            && cache
                .keys
                .iter()
                .all(|&(key, name)| table.get(key).is_some_and(|entry| entry.name == name));
        if !valid {
            cache.len = table.len();
            cache.keys = table
                .entries()
                .filter_map(|entry| {
                    IDLE_RNG.iter().find(|&&name| name == entry.name).map(|&name| (entry.key, name))
                })
                .collect();
        }
        cache
            .keys
            .iter()
            .map(|&(key, _)| machine.hook_stat(key).estimated_rust_instructions())
            .sum()
    })
}

/// The hook keys of the idle RNG routines in the table `len` routines long that was seen last.
#[derive(Default)]
struct IdleRngKeys {
    len: usize,
    keys: Vec<(u32, &'static str)>,
}

thread_local! {
    static IDLE_RNG_KEYS: std::cell::RefCell<IdleRngKeys> =
        std::cell::RefCell::new(IdleRngKeys::default());
}

/// Whether `machine` runs routines in Rust while none of them has a measured mean (from a `Verify`
/// run, [`HookStats::mean_instructions`]): their runs then stand for no instructions, so the
/// coverage figures are not known. A machine that runs nothing in Rust knows its share (0%).
#[must_use]
pub fn means_missing(machine: &Machine) -> bool {
    let Some(table) = machine.hook_table() else { return false };
    let in_rust = |key: u32| table.get(key).is_some_and(|entry| entry.mode == HookMode::Rust);
    // Only a routine that has run, or has a mean given, has counters: look at those, not at every
    // routine of the table (a recompiled build's has tens of thousands).
    table.entries().any(|entry| entry.mode == HookMode::Rust)
        && !machine
            .hook_counters()
            .any(|(key, stats)| stats.mean_instructions > 0.0 && in_rust(key))
}

/// One routine of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    /// Its hook key (its address), which names it in a mode change.
    pub key: u32,
    /// Its mode now: original where the machine has no table or no entry for it.
    pub mode: HookMode,
    pub calls: u64,
    /// The calls that fell back to the original, in percent of the calls; `None` before a call.
    pub fallback: Option<f64>,
    pub divergences: u64,
}

/// The rows of the routines `listed` (name and hook key, in order), with their modes in `table`
/// and their counters in `stats`. ([`rows_of`] does the same for a machine without copying its counters;
/// this is its specification, which the tests check it against.)
#[cfg(test)]
#[must_use]
pub fn rows<'a>(
    listed: impl IntoIterator<Item = (&'a str, u32)>,
    table: Option<&ssd_oracle::HookTable>,
    stats: &[(String, ssd_oracle::HookStats)],
) -> Vec<Row> {
    listed
        .into_iter()
        .map(|(name, key)| {
            let mode =
                table.and_then(|table| table.get(key)).map_or(HookMode::Original, |e| e.mode);
            let stats = stats
                .iter()
                .find(|(stat_name, _)| stat_name.as_str() == name)
                .map(|(_, stats)| *stats)
                .unwrap_or_default();
            Row {
                name: name.to_owned(),
                key,
                mode,
                calls: stats.calls,
                fallback: (stats.calls > 0)
                    .then(|| 100.0 * stats.timing_fallback as f64 / stats.calls as f64),
                divergences: stats.divergences,
            }
        })
        .collect()
}

/// The rows of the machine's routines: the routines of the symbols file when there is one, else
/// the entries of its table. A routine's counters are looked up by its key, so a table of tens of
/// thousands of routines costs only the rows that are listed.
#[must_use]
pub fn rows_of(machine: &Machine, symbols: Option<&Symbols>) -> Vec<Row> {
    let table = machine.hook_table();
    let listed: Vec<(&str, u32)> = match symbols {
        Some(symbols) => symbols
            .routines()
            .iter()
            .map(|routine| (routine.name.as_str(), routine.address))
            .collect(),
        None => table
            .map(|table| table.entries().map(|e| (e.name.as_str(), e.key)).collect())
            .unwrap_or_default(),
    };
    listed
        .into_iter()
        .map(|(name, key)| {
            let entry = table.and_then(|table| table.get(key));
            // The counters of a routine the table holds; the ones of a routine it does not hold
            // are none (the table's names are the symbols').
            let stats = entry.map(|entry| machine.hook_stat(entry.key)).unwrap_or_default();
            Row {
                name: name.to_owned(),
                key,
                mode: entry.map_or(HookMode::Original, |e| e.mode),
                calls: stats.calls,
                fallback: (stats.calls > 0)
                    .then(|| 100.0 * stats.timing_fallback as f64 / stats.calls as f64),
                divergences: stats.divergences,
            }
        })
        .collect()
}

/// The two figures of the summary: the share of the window's instructions that ran as Rust, and the
/// same without the idle RNG, in percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub total: f64,
    pub without_rng: f64,
}

/// The summary of a window: `coverage` is its instructions, `idle_rng` the idle RNG's part of the
/// Rust ones.
#[must_use]
pub fn summary(coverage: Coverage, idle_rng: u64) -> Summary {
    Summary { total: coverage.percent(), without_rng: coverage.excluding(idle_rng).percent() }
}

/// The summary of the window of `history`, or `None` while there is nothing to show: no frame yet,
/// or the means are missing ([`means_missing`]).
#[must_use]
pub fn window_summary(history: &FrameHistory, means_missing: bool) -> Option<Summary> {
    (!means_missing && !history.is_empty()).then(|| summary(history.coverage(), history.idle_rng()))
}

/// The modes of `rows` after the routine with hook key `key` is set to `mode`; `None` when that
/// changes nothing (no such routine, or it has the mode already).
#[must_use]
pub fn mode_change(rows: &[Row], key: u32, mode: HookMode) -> Option<Vec<(String, HookMode)>> {
    rows.iter().find(|row| row.key == key && row.mode != mode)?;
    Some(
        rows.iter()
            .map(|row| (row.name.clone(), if row.key == key { mode } else { row.mode }))
            .collect(),
    )
}

/// The global mode when every routine of `modes` has the same one, else `None` (they differ).
#[must_use]
pub fn uniform_mode(modes: &[(String, HookMode)]) -> Option<HooksArg> {
    let mut all = modes.iter().map(|(_, mode)| *mode);
    let first = all.next().unwrap_or(HookMode::Original);
    all.all(|mode| mode == first).then(|| HooksArg::of(first))
}

/// What the Routines tab shows this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The window's figures; `None` shows a dash ([`window_summary`]).
    pub summary: Option<Summary>,
    /// The means are missing: the summary says what to do ([`MEANS_HINT`]).
    pub means_missing: bool,
    pub rows: Vec<Row>,
    /// The bars of the frame history ([`FrameHistory::bars`]).
    pub bars: Vec<Option<f64>>,
}

/// The view of the tab, from the machine, its symbols and the frame history.
#[must_use]
pub fn view(machine: &Machine, symbols: Option<&Symbols>, history: &FrameHistory) -> View {
    let missing = means_missing(machine);
    View {
        summary: window_summary(history, missing),
        means_missing: missing,
        rows: rows_of(machine, symbols),
        bars: history.bars(),
    }
}

// --- drawing ----------------------------------------------------------------------------------

/// Width of the mode switch, and of the calls and fallback columns; the gaps between columns.
const MODE_WIDTH: f32 = 104.0;
const CALLS_WIDTH: f32 = 72.0;
const FALLBACK_WIDTH: f32 = 56.0;
const GAP: f32 = 8.0;
/// Height of a routine's row, and of the footer.
const ROW_HEIGHT: f32 = 36.0;
const FOOTER_HEIGHT: f32 = 40.0;

/// The rectangles of a row's columns: name, mode, calls, fallback. `row` is the whole row; the
/// columns sit 8 points in from its ends, as the table's padding.
fn columns(row: Rect) -> [Rect; 4] {
    let inner = row.shrink2(vec2(8.0, 0.0));
    let fallback = Rect::from_min_max(pos2(inner.max.x - FALLBACK_WIDTH, inner.min.y), inner.max);
    let calls_right = fallback.min.x - GAP;
    let calls = Rect::from_min_max(
        pos2(calls_right - CALLS_WIDTH, inner.min.y),
        pos2(calls_right, inner.max.y),
    );
    let mode_right = calls.min.x - GAP;
    let mode = Rect::from_min_max(
        pos2(mode_right - MODE_WIDTH, inner.min.y),
        pos2(mode_right, inner.max.y),
    );
    let name = Rect::from_min_max(inner.min, pos2(mode.min.x - GAP, inner.max.y));
    [name, mode, calls, fallback]
}

/// Draws the tab. Mode changes are pushed to `actions`.
pub fn show(ui: &mut Ui, view: &View, controls: &Controls, actions: &mut Vec<PanelAction>) {
    summary_section(ui, view);
    Frame::new().inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        header(ui);
    });
    let height = (ui.available_height() - FOOTER_HEIGHT).max(ROW_HEIGHT);
    ScrollArea::vertical()
        .id_salt("routine-rows")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if view.rows.is_empty() {
                    ui.label(
                        RichText::new("No routines to list: the symbols file could not be read.")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
                for row in &view.rows {
                    routine_row(ui, row, controls, actions);
                }
            });
        });
    footer(ui);
}

/// A line under the panel's tab strip: a hairline across the panel.
fn hairline(ui: &mut Ui) {
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, LINE);
}

fn summary_section(ui: &mut Ui, view: &View) {
    Frame::new().inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 14 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Instructions run in Rust").size(12.0).color(MUTED_LIGHT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The side panel sets the spacing to zero; the figures need a gap.
                ui.spacing_mut().item_spacing.x = 6.0;
                let mono = |text: String, color: Color32| {
                    RichText::new(text).font(theme::mono(12.0)).color(color)
                };
                let (without, total) = match view.summary {
                    Some(figures) => (
                        format!("{:.2}% without idle RNG", figures.without_rng),
                        format!("{:.1}%", figures.total),
                    ),
                    None => ("\u{2014} without idle RNG".to_owned(), "\u{2014}".to_owned()),
                };
                ui.label(mono(without, MUTED));
                ui.label(mono("\u{b7}".to_owned(), MUTED));
                ui.label(mono(total, TEXT));
            });
        });
        if view.means_missing {
            ui.label(RichText::new(MEANS_HINT).size(11.0).color(MUTED));
        }
        ui.add_space(10.0);
        bars(ui, &view.bars);
        ui.add_space(8.0);
        let divergences: u64 = view.rows.iter().map(|row| row.divergences).sum();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            ui.label(RichText::new("last 60 frames").size(11.0).color(MUTED));
            let routines = view.rows.len();
            ui.label(RichText::new(format!("{routines} routines")).size(11.0).color(MUTED));
            let (text, color) = if divergences > 0 {
                (format!("{divergences} divergences"), DIVERGE)
            } else {
                ("0 divergences".to_owned(), TEXT)
            };
            ui.label(RichText::new(text).size(11.0).color(color));
        });
    });
    hairline(ui);
}

/// The frame strip: one bar per frame, the newest in orange; the tallest is the window's largest
/// share (at least 1%), so the bars show how the frames compare with each other.
fn bars(ui: &mut Ui, bars: &[Option<f64>]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let peak = bars.iter().flatten().fold(1.0_f64, |peak, &share| peak.max(share));
    let count = bars.len().max(1) as f32;
    let gap = 2.0;
    let width = (rect.width() - gap * (count - 1.0)) / count;
    let painter = ui.painter();
    for (i, bar) in bars.iter().enumerate() {
        let Some(share) = bar else { continue };
        let height = ((share / peak) as f32 * rect.height()).max(1.0);
        let left = rect.min.x + i as f32 * (width + gap);
        let bar_rect =
            Rect::from_min_max(pos2(left, rect.max.y - height), pos2(left + width, rect.max.y));
        let color = if i + 1 == bars.len() { ACCENT } else { BAR };
        painter.rect_filled(bar_rect, 1.0, color);
    }
}

fn header(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let [name, mode, calls, fallback] = columns(rect);
    let painter = ui.painter();
    let font = FontId::proportional(11.0);
    painter.text(name.left_center(), Align2::LEFT_CENTER, "Routine", font.clone(), MUTED);
    painter.text(mode.center(), Align2::CENTER_CENTER, "Mode", font.clone(), MUTED);
    painter.text(calls.right_center(), Align2::RIGHT_CENTER, "Calls", font.clone(), MUTED);
    painter.text(fallback.right_center(), Align2::RIGHT_CENTER, "Fallback", font, MUTED);
}

fn routine_row(ui: &mut Ui, row: &Row, controls: &Controls, actions: &mut Vec<PanelAction>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let painter = ui.painter().clone();
    if row.divergences > 0 {
        painter.rect_filled(rect, CornerRadius::same(6), DIVERGE_FILL);
    }
    let [name, mode, calls, fallback] = columns(rect);
    let font = theme::mono(12.0);
    let shown = theme::fit_text(&painter, &row.name, &font, name.width());
    painter.text(name.left_center(), Align2::LEFT_CENTER, shown, font.clone(), TEXT);
    mode_switch(ui, row, mode, controls, actions);
    painter.text(
        calls.right_center(),
        Align2::RIGHT_CENTER,
        grouped(row.calls),
        font.clone(),
        TEXT_STRONG,
    );
    let (text, color) = match row.fallback {
        Some(share) if share > FALLBACK_ALERT => (format!("{share:.1}%"), ACCENT),
        Some(share) => (format!("{share:.1}%"), TEXT_STRONG),
        None => ("-".to_owned(), DISABLED),
    };
    painter.text(fallback.right_center(), Align2::RIGHT_CENTER, text, font, color);
}

/// The O / R / V switch of a row, 22 points high. Clicking a letter sets the routine's mode; the
/// switch is locked (with the reason as a tooltip) when the hook mode cannot change.
fn mode_switch(
    ui: &mut Ui,
    row: &Row,
    column: Rect,
    controls: &Controls,
    actions: &mut Vec<PanelAction>,
) {
    let switch = Rect::from_center_size(column.center(), vec2(column.width(), 22.0));
    let locked = match controls.hooks_switch {
        HooksSwitch::Allowed => None,
        HooksSwitch::Locked(why) => Some(why),
    };
    let cell_width = switch.width() / 3.0;
    for (i, (mode, letter)) in
        [(HookMode::Original, "O"), (HookMode::Rust, "R"), (HookMode::Verify, "V")]
            .into_iter()
            .enumerate()
    {
        let cell = Rect::from_min_size(
            switch.min + vec2(cell_width * i as f32, 0.0),
            vec2(cell_width, switch.height()),
        );
        let response = ui.interact(cell, Id::new(("routine-mode", row.key, i)), Sense::click());
        let on = row.mode == mode;
        let (fill, color) = mode_look(mode, on, response.hovered() && locked.is_none());
        let painter = ui.painter();
        painter.rect_filled(cell, 0.0, fill);
        painter.text(cell.center(), Align2::CENTER_CENTER, letter, theme::mono(11.0), color);
        if let Some(why) = locked {
            response.on_hover_text(why);
        } else {
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if response.clicked() && !on {
                actions.push(PanelAction::SetRoutineMode { key: row.key, mode });
            }
        }
    }
    ui.painter().rect_stroke(switch, 5.0, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
}

/// The fill and the letter colour of a mode cell: the mode a row has is lit in its own colour
/// (Rust orange, Verify blue, Original bright), the others are dim.
fn mode_look(mode: HookMode, on: bool, hovered: bool) -> (Color32, Color32) {
    match (mode, on) {
        (HookMode::Original, true) => (ORIGINAL_FILL, BRIGHT),
        (HookMode::Rust, true) => (RUST_FILL, ACCENT),
        (HookMode::Verify, true) => (VERIFY_FILL, VERIFY),
        _ if hovered => (CONTROL_HOVER, MUTED_LIGHT),
        _ => (CONTROL, DISABLED),
    }
}

fn footer(ui: &mut Ui) {
    hairline(ui);
    Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("O original \u{b7} R Rust \u{b7} V verify").size(11.0).color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(
                    RichText::new("Fallback: original ran near an interrupt")
                        .size(11.0)
                        .color(MUTED),
                );
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook_setup::HookSwitch;
    use crate::test_support::sram_counter_machine;
    use ssd_oracle::{HookAction, HookEntry, HookStats, HookTable, Joypad, ReturnKind};
    use std::path::Path;

    fn counters(calls: u64, fallback: u64, divergences: u64) -> HookStats {
        HookStats { calls, timing_fallback: fallback, divergences, ..HookStats::default() }
    }

    fn frame(number: u64, original: u64, rust: u64) -> FrameStats {
        FrameStats {
            frame: number,
            original_instructions: original,
            rust_instructions_estimated: rust,
            ..FrameStats::default()
        }
    }

    fn row(name: &str, key: u32, mode: HookMode) -> Row {
        Row { name: name.to_owned(), key, mode, calls: 0, fallback: None, divergences: 0 }
    }

    #[test]
    fn a_routine_row_has_its_calls_fallback_and_divergences_and_is_original_without_a_table() {
        let listed =
            [("rng_next_a", 0x8000_u32), ("bin24_to_packed_bcd", 0x9000), ("unused", 0xA000)];
        let counts = [
            ("rng_next_a".to_owned(), counters(200, 30, 0)),
            ("bin24_to_packed_bcd".to_owned(), counters(29, 0, 2)),
        ];
        let table_rows = rows(listed, None, &counts);
        assert_eq!(table_rows.len(), 3);
        assert_eq!(table_rows[0].mode, HookMode::Original, "no table: original");
        assert_eq!(table_rows[0].calls, 200);
        assert!((table_rows[0].fallback.unwrap() - 15.0).abs() < 1e-9, "30 of 200 calls");
        assert_eq!(table_rows[1].divergences, 2);
        assert_eq!(table_rows[1].fallback, Some(0.0), "no fallback of 29 calls");
        assert_eq!(table_rows[2], row("unused", 0xA000, HookMode::Original));
    }

    #[test]
    fn the_mode_of_a_row_is_the_table_entry_of_its_key() {
        fn never(_: &mut ssd_oracle::Ctx<'_>) {}
        let entry = |name: &str, key, mode| HookEntry {
            name: name.to_owned(),
            key,
            mode,
            action: HookAction::Routine { func: never, ret: ReturnKind::Rts },
            entry_m8: None,
            entry_x8: None,
            flags_mask: 0xFF,
            verify_sample: 1,
            bound: None,
            budget_frames: None,
            cycles: None,
        };
        let table: HookTable =
            [entry("a", 0x8000, HookMode::Rust), entry("b", 0x9000, HookMode::Verify)]
                .into_iter()
                .collect();
        let table_rows = rows([("a", 0x8000), ("b", 0x9000), ("c", 0xA000)], Some(&table), &[]);
        let modes: Vec<HookMode> = table_rows.iter().map(|row| row.mode).collect();
        assert_eq!(modes, [HookMode::Rust, HookMode::Verify, HookMode::Original]);
    }

    #[test]
    fn the_summary_is_the_window_and_the_window_without_the_idle_rng() {
        // 600 Rust of 2400 instructions: 25%. The RNGs stand for 500 of the 600: without them, 100
        // of 1900, about 5.26%.
        let coverage = Coverage { original_instructions: 1800, rust_instructions_estimated: 600 };
        let figures = summary(coverage, 500);
        assert!((figures.total - 25.0).abs() < 1e-9, "{figures:?}");
        assert!((figures.without_rng - 100.0 * 100.0 / 1900.0).abs() < 1e-9, "{figures:?}");
        let nothing = summary(Coverage::default(), 0);
        assert_eq!((nothing.total, nothing.without_rng), (0.0, 0.0), "no instructions, no share");
    }

    #[test]
    fn the_window_sums_the_last_sixty_frames_and_their_idle_rng_shares() {
        let mut history = FrameHistory::default();
        let mut total = 0;
        // Frame n: 100 original instructions and n Rust ones, of which the idle RNG has n (the
        // counters grow by n in frame n).
        for number in 1..=(HISTORY as u64 + 10) {
            total += number;
            history.record(Some(frame(number, 100, number)), total);
        }
        // Frames 11 to 70 remain.
        let kept: u64 = (11..=(HISTORY as u64 + 10)).sum();
        assert_eq!(
            history.coverage(),
            Coverage {
                original_instructions: 100 * HISTORY as u64,
                rust_instructions_estimated: kept
            }
        );
        assert_eq!(history.idle_rng(), kept);
        // The idle RNG is all of the Rust side here: 2430 of 2430, so without it nothing is left.
        let figures = summary(history.coverage(), history.idle_rng());
        assert!((figures.total - 100.0 * 2430.0 / 8430.0).abs() < 1e-9, "{figures:?}");
        assert_eq!(figures.without_rng, 0.0, "{figures:?}");
    }

    #[test]
    fn a_frame_noted_twice_does_not_move_the_idle_rng_base() {
        let mut history = FrameHistory::default();
        history.record(Some(frame(1, 90, 10)), 300);
        history.record(Some(frame(1, 90, 10)), 900);
        history.record(Some(frame(2, 90, 10)), 450);
        assert_eq!(history.idle_rng(), 300 + 150);
    }

    #[test]
    fn the_idle_rng_share_restarts_after_a_rebase_or_a_reset_of_the_counters() {
        let mut history = FrameHistory::default();
        history.record(Some(frame(1, 90, 10)), 100);
        // The machine's table changed at a total of 1000: the next frame counts from there.
        history.rebase(1000);
        history.record(Some(frame(2, 90, 10)), 1040);
        // A total below the base: the counters started again from zero.
        history.record(Some(frame(3, 90, 10)), 25);
        assert_eq!(history.idle_rng(), 100 + 40 + 25);
    }

    #[test]
    fn the_window_figures_wait_for_a_frame_and_for_the_means_of_the_routines_in_rust() {
        let mut history = FrameHistory::default();
        assert_eq!(window_summary(&history, false), None, "no frame yet");
        history.record(Some(frame(1, 90, 10)), 0);
        assert!(window_summary(&history, false).is_some());
        assert_eq!(window_summary(&history, true), None, "the means are missing");
    }

    #[test]
    fn the_means_are_missing_while_a_routine_in_rust_has_none() {
        let symbols = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let switch = HookSwitch::load(Some(Path::new(symbols)), None).unwrap();
        let (mut machine, _store) = sram_counter_machine();
        assert!(!means_missing(&machine), "no table: nothing runs in Rust");
        switch.apply(&mut machine, HooksArg::Rust).unwrap();
        assert!(means_missing(&machine), "routines in Rust, none measured");
        machine.set_mean_instructions(&[("rng_next_a".to_owned(), 11.0)]);
        assert!(!means_missing(&machine), "one measured mean is enough");
        switch.apply(&mut machine, HooksArg::Original).unwrap();
        assert!(!means_missing(&machine), "nothing runs in Rust again");
    }

    #[test]
    fn the_rows_of_a_machine_are_the_rows_of_its_copied_counters() {
        let symbols_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let switch = HookSwitch::load(Some(Path::new(symbols_path)), None).unwrap();
        let (mut machine, _store) = sram_counter_machine();
        let symbols = switch.symbols().as_ref();
        let listed = || symbols.routines().iter().map(|r| (r.name.as_str(), r.address));
        assert_eq!(rows_of(&machine, Some(symbols)), rows(listed(), None, &[]), "no table");
        switch.apply(&mut machine, HooksArg::Rust).unwrap();
        for _ in 0..3 {
            machine.run_frame(Joypad::default(), &mut ssd_oracle::NullAudio).unwrap();
        }
        let wanted = rows(listed(), machine.hook_table(), &machine.hook_stats());
        assert_eq!(rows_of(&machine, Some(symbols)), wanted, "the table's routines in Rust");
        let table_rows = rows_of(&machine, None);
        assert_eq!(table_rows.len(), machine.hook_table().unwrap().len(), "no symbols: the table");
    }

    #[test]
    fn the_idle_rng_total_follows_the_counters_of_the_table_the_machine_has() {
        let symbols_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let switch = HookSwitch::load(Some(Path::new(symbols_path)), None).unwrap();
        let (mut machine, _store) = sram_counter_machine();
        assert_eq!(idle_rng_total(&machine), 0, "no table");
        switch.apply(&mut machine, HooksArg::Rust).unwrap();
        let by_names = |machine: &Machine| -> u64 {
            machine
                .hook_stats()
                .iter()
                .filter(|(name, _)| IDLE_RNG.contains(&name.as_str()))
                .map(|(_, stats)| stats.estimated_rust_instructions())
                .sum()
        };
        assert_eq!(idle_rng_total(&machine), by_names(&machine));
        switch.apply(&mut machine, HooksArg::Original).unwrap();
        assert_eq!(idle_rng_total(&machine), 0, "the original code again");
    }

    #[test]
    fn a_mode_change_is_the_new_modes_or_nothing_when_nothing_changes() {
        let rows = [row("a", 0x8000, HookMode::Original), row("b", 0x9000, HookMode::Rust)];
        assert_eq!(
            mode_change(&rows, 0x8000, HookMode::Verify),
            Some(vec![("a".to_owned(), HookMode::Verify), ("b".to_owned(), HookMode::Rust)])
        );
        assert_eq!(mode_change(&rows, 0x9000, HookMode::Rust), None, "already that mode");
        assert_eq!(mode_change(&rows, 0x1234, HookMode::Rust), None, "no such routine");
    }

    #[test]
    fn the_global_mode_is_shown_only_while_the_routines_agree() {
        let same = [("a".to_owned(), HookMode::Rust), ("b".to_owned(), HookMode::Rust)];
        assert_eq!(uniform_mode(&same), Some(HooksArg::Rust));
        let mixed = [("a".to_owned(), HookMode::Rust), ("b".to_owned(), HookMode::Verify)];
        assert_eq!(uniform_mode(&mixed), None);
        assert_eq!(uniform_mode(&[]), Some(HooksArg::Original), "no routine, no mix");
    }

    #[test]
    fn the_frame_history_keeps_the_last_sixty_frames_once_each() {
        let mut history = FrameHistory::default();
        assert_eq!(history.bars(), vec![None; HISTORY]);
        history.record(None, 0);
        history.record(Some(frame(1, 90, 10)), 0);
        history.record(Some(frame(1, 90, 10)), 0);
        let bars = history.bars();
        assert_eq!(bars.len(), HISTORY);
        assert_eq!(bars.iter().flatten().count(), 1, "frame 1 once");
        assert!((bars[HISTORY - 1].unwrap() - 10.0).abs() < 1e-9, "the newest is last");
        for number in 2..=(HISTORY as u64 + 5) {
            history.record(Some(frame(number, 90, 10)), 0);
        }
        assert_eq!(history.bars().iter().flatten().count(), HISTORY, "a full strip");
        assert_eq!(history.frames.front().map(|s| s.stats.frame), Some(6), "the oldest ones went");
    }
}
