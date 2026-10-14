//! The replays tab: recording, the list of replay files with Play and Verify, the progress of a
//! verification run, and the divergence reports with Reproduce and Open state. Thin on purpose:
//! the state and what it means are in `replays.rs`; this draws them and pushes the
//! [`ReplayAction`]s the user asks for.

use crate::panel::{PanelAction, ReplayAction, grouped};
use crate::replays::{ReplaysState, Verification};
use crate::theme::{
    self, ACCENT, GROUND, LINE, LINE_STRONG, MUTED, MUTED_LIGHT, TEXT, TEXT_STRONG, VERIFY,
    VERIFY_FILL, VERIFY_LINE, VERIFY_ROW,
};
use crate::ui::{button, small_button};
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense,
    Stroke, Ui, vec2,
};

/// Draws the replays tab into `ui`.
pub fn draw(ui: &mut Ui, replays: &ReplaysState, actions: &mut Vec<PanelAction>) {
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            toolbar(ui, replays, actions);
            list(ui, replays, actions);
            verification(ui, replays, actions);
            divergences(ui, replays, actions);
        });
    });
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).size(12.0).color(MUTED_LIGHT));
}

fn toolbar(ui: &mut Ui, replays: &ReplaysState, actions: &mut Vec<PanelAction>) {
    let font = FontId::proportional(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if replays.recording.is_some() {
            if button(ui, "Stop recording", font.clone(), None, true, Some(ACCENT)) {
                actions.push(PanelAction::Replay(ReplayAction::StopRecording));
            }
        } else if button(ui, "Record new replay", font.clone(), None, true, Some(ACCENT)) {
            actions.push(PanelAction::Replay(ReplayAction::Record));
        }
        if replays.playing.is_some()
            && button(ui, "Stop replay", font.clone(), None, true, Some(ACCENT))
        {
            actions.push(PanelAction::Replay(ReplayAction::StopPlayback));
        }
        if button(ui, "Refresh", font, None, true, None) {
            actions.push(PanelAction::Replay(ReplayAction::Refresh));
        }
    });
    if let Some(name) = &replays.recording {
        let note = format!("Recording {name}: the file is written when you stop it.");
        ui.label(RichText::new(note).size(11.0).color(VERIFY));
    }
    if let Some(name) = &replays.playing {
        let note =
            format!("Playing {name}: loading waits for its end; freezes wait for Stop replay.");
        ui.label(RichText::new(note).size(11.0).color(VERIFY));
    }
    if let Some(notice) = &replays.notice {
        ui.label(RichText::new(notice).size(11.0).color(ACCENT));
    }
}

/// The replay files, each with its frames, its last outcome and the Play and Verify buttons.
fn list(ui: &mut Ui, replays: &ReplaysState, actions: &mut Vec<PanelAction>) {
    section(ui, "Replays in .work/replays");
    if replays.replays.is_empty() {
        let note = "No replay files yet: record one, or copy .ssdreplay files there.";
        ui.label(RichText::new(note).size(11.0).color(MUTED));
    }
    for (row, entry) in replays.replays.iter().enumerate() {
        let running = replays
            .verification
            .as_ref()
            .is_some_and(|run| run.running() && run.name == entry.name);
        let fill = if running { VERIFY_ROW } else { Color32::TRANSPARENT };
        Frame::new()
            .fill(fill)
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&entry.name).font(theme::mono(12.0)).color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (text, color) = if running {
                            ("verifying".to_owned(), VERIFY)
                        } else {
                            match &entry.outcome {
                                Some(outcome) => (outcome.text(), TEXT_STRONG),
                                None => ("not verified".to_owned(), MUTED),
                            }
                        };
                        ui.label(RichText::new(text).size(11.0).color(color));
                    });
                });
                match &entry.frames {
                    Ok(frames) => {
                        let info = format!("{} frames", grouped(*frames));
                        ui.label(RichText::new(info).size(11.0).color(MUTED));
                    }
                    Err(reason) => {
                        ui.label(RichText::new(reason).size(11.0).color(ACCENT));
                    }
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let playable = entry.frames.is_ok();
                    if small_button(ui, "Play", playable) {
                        actions.push(PanelAction::Replay(ReplayAction::Play(row)));
                    }
                    if small_button(ui, "Verify", playable) {
                        actions.push(PanelAction::Replay(ReplayAction::Verify(row)));
                    }
                });
            });
    }
}

/// The verification run: its progress, its counters and its controls, while it runs or after.
fn verification(ui: &mut Ui, replays: &ReplaysState, actions: &mut Vec<PanelAction>) {
    let Some(run) = &replays.verification else { return };
    let running = run.running();
    let (fill, line) = if running { (VERIFY_FILL, VERIFY_LINE) } else { (GROUND, LINE) };
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            heading(ui, run);
            progress_bar(ui, run.progress());
            counters(ui, run);
            if let Some(outcome) = &run.outcome {
                ui.label(RichText::new(outcome.text()).size(12.0).color(TEXT_STRONG));
            }
            if running {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let font = FontId::proportional(12.0);
                    let pause = if run.paused { "Continue" } else { "Pause" };
                    if button(ui, pause, font.clone(), None, true, None) {
                        actions.push(PanelAction::Replay(ReplayAction::TogglePauseVerify));
                    }
                    if button(ui, "Stop", font, None, true, None) {
                        actions.push(PanelAction::Replay(ReplayAction::StopVerify));
                    }
                });
            }
        });
}

fn heading(ui: &mut Ui, run: &Verification) {
    let title = match (run.running(), run.paused) {
        (false, _) => format!("Verified {}", run.name),
        (true, true) => format!("Paused: {}", run.name),
        (true, false) => format!("Verifying {}", run.name),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(12.0).color(VERIFY));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let checks = format!("{} checks", grouped(run.report.events));
            ui.label(RichText::new(checks).font(theme::mono(12.0)).color(TEXT_STRONG));
        });
    });
}

fn progress_bar(ui: &mut Ui, progress: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, GROUND);
    let done = rect.with_max_x(rect.min.x + rect.width() * progress.clamp(0.0, 1.0));
    ui.painter().rect_filled(done, 3.0, VERIFY);
}

/// Differences, cycle drift and frames, the three counters of a run.
fn counters(ui: &mut Ui, run: &Verification) {
    let frames = format!("{} / {}", grouped(run.report.frames), grouped(run.frames_total));
    let cells = [
        ("differences", grouped(run.differences())),
        ("cycle drift", run.report.max_cycle_drift.unsigned_abs().to_string()),
        ("frames", frames),
    ];
    ui.columns(3, |columns| {
        for (column, (label, value)) in columns.iter_mut().zip(cells) {
            column.label(RichText::new(label).size(11.0).color(MUTED));
            column.label(RichText::new(value).font(theme::mono(13.0)).color(TEXT));
        }
    });
}

/// The divergence reports: each with its lines, what Reproduce said, and the two buttons.
fn divergences(ui: &mut Ui, replays: &ReplaysState, actions: &mut Vec<PanelAction>) {
    section(ui, "Divergences");
    if replays.reports.is_empty() {
        let note = "Shown when a check fails; none so far.";
        ui.label(RichText::new(note).size(11.0).color(MUTED));
    }
    for (row, report) in replays.reports.iter().enumerate() {
        Frame::new()
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, LINE_STRONG))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(&report.title).font(theme::mono(12.0)).color(ACCENT));
                for line in &report.lines {
                    ui.label(RichText::new(line).size(11.0).color(MUTED_LIGHT));
                }
                if let Some(verdict) = &report.verdict {
                    ui.label(RichText::new(verdict).size(11.0).color(TEXT_STRONG));
                }
                if report.header.is_none() {
                    let note = "A replay check has no routine to run again: use Open state.";
                    ui.label(RichText::new(note).size(11.0).color(MUTED));
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let runnable = report.header.is_some();
                    if small_button(ui, "Reproduce", runnable) {
                        actions.push(PanelAction::Replay(ReplayAction::Reproduce(row)));
                    }
                    if small_button(ui, "Open state", true) {
                        actions.push(PanelAction::Replay(ReplayAction::OpenState(row)));
                    }
                });
            });
    }
}
