//! The debugger tab: the stop banner, the stop-at-interrupts switch, the registers and flags, the
//! breakpoints and the recent steps. Thin on purpose: what the lines mean is in `debugger.rs`;
//! this draws them and pushes the [`DebugAction`]s the user asks for.

use crate::debugger::{DebugState, flag_bits, register_rows};
use crate::panel::{DebugAction, PanelAction};
use crate::theme::{
    self, ACCENT, ACCENT_FILL, BANNER_LABEL, CONTROL, DISABLED, DISABLED_LINE, GROUND, MUTED,
    MUTED_LIGHT, TEXT, TEXT_STRONG, TOAST_FILL, TOAST_LINE,
};
use crate::ui::{View, button, small_button};
use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Key, Label, Layout, Margin, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Ui, vec2,
};

/// Draws the debugger tab into `ui`.
pub fn draw(ui: &mut Ui, view: &View<'_>, debug: &mut DebugState, actions: &mut Vec<PanelAction>) {
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            banner(ui, debug);
            interrupts(ui, debug, actions);
            registers(ui, view);
            breakpoints(ui, view, debug, actions);
            recent_steps(ui, debug);
        });
    });
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).size(12.0).color(MUTED_LIGHT));
}

/// Where the run stopped and what the next step does, when it stopped.
fn banner(ui: &mut Ui, debug: &DebugState) {
    let Some(banner) = &debug.banner else { return };
    Frame::new()
        .fill(TOAST_FILL)
        .stroke(Stroke::new(1.0, TOAST_LINE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new(&banner.label).size(11.0).color(BANNER_LABEL));
            ui.label(RichText::new(&banner.title).font(theme::mono(14.0)).color(ACCENT));
            if !banner.detail.is_empty() {
                ui.label(RichText::new(&banner.detail).size(11.0).color(MUTED_LIGHT));
            }
        });
}

fn interrupts(ui: &mut Ui, debug: &DebugState, actions: &mut Vec<PanelAction>) {
    let mut value = debug.stop_at_interrupts;
    let label = RichText::new("Stop at interrupts").size(12.0).color(MUTED_LIGHT);
    if ui.checkbox(&mut value, label).changed() {
        actions.push(PanelAction::Debug(DebugAction::ToggleStopAtInterrupts));
    }
}

fn registers(ui: &mut Ui, view: &View<'_>) {
    section(ui, "Registers");
    let rows = register_rows(view.registers);
    for chunk in rows.chunks(3) {
        ui.columns(3, |columns| {
            for (column, (name, value)) in columns.iter_mut().zip(chunk) {
                register_cell(column, name, value);
            }
        });
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (name, set) in flag_bits(view.registers) {
            let (rect, _) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::hover());
            let (fill, color) = if set { (ACCENT_FILL, ACCENT) } else { (CONTROL, DISABLED) };
            ui.painter().rect_filled(rect, 4.0, fill);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, name, theme::mono(12.0), color);
        }
    });
}

/// One register: its name on the left, its value on the right, in a bordered cell.
fn register_cell(ui: &mut Ui, name: &str, value: &str) {
    Frame::new()
        .fill(GROUND)
        .stroke(Stroke::new(1.0, DISABLED_LINE))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).font(theme::mono(12.0)).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(value).font(theme::mono(12.0)).color(TEXT));
                });
            });
        });
}

fn breakpoints(
    ui: &mut Ui,
    view: &View<'_>,
    debug: &mut DebugState,
    actions: &mut Vec<PanelAction>,
) {
    section(ui, "Breakpoints");
    // A recording runs without hooks, and a breakpoint is one: none can be set meanwhile.
    let editable = !view.recording;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let width = (ui.available_width() - 64.0).max(80.0);
        let field = ui.add_enabled(
            editable,
            TextEdit::singleline(&mut debug.target)
                .hint_text("routine or $address")
                .desired_width(width),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let clicked = button(ui, "Add", FontId::proportional(12.0), Some(56.0), editable, None);
        if editable && (clicked || entered) {
            actions.push(PanelAction::Debug(DebugAction::AddBreakpoint));
        }
    });
    if view.recording {
        let note = "Breakpoints wait for the recording to end: a recording runs without hooks.";
        ui.label(RichText::new(note).size(11.0).color(MUTED));
    }
    if let Some(notice) = &debug.notice {
        ui.label(RichText::new(notice).size(11.0).color(ACCENT));
    }
    for (index, breakpoint) in debug.breakpoints.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let mut enabled = breakpoint.enabled;
            if ui.checkbox(&mut enabled, "").changed() {
                actions.push(PanelAction::Debug(DebugAction::ToggleBreakpoint(index)));
            }
            ui.label(RichText::new(&breakpoint.name).font(theme::mono(12.0)).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_button(ui, "Remove", true) {
                    actions.push(PanelAction::Debug(DebugAction::RemoveBreakpoint(index)));
                }
                let note = if breakpoint.hits > 0 {
                    format!("hit {}", breakpoint.hits)
                } else if !breakpoint.enabled {
                    "off".to_owned()
                } else {
                    String::new()
                };
                ui.label(RichText::new(note).size(11.0).color(MUTED));
            });
        });
    }
}

fn recent_steps(ui: &mut Ui, debug: &DebugState) {
    section(ui, "Recent steps");
    if debug.log.is_empty() {
        ui.label(RichText::new("Nothing yet.").size(11.0).color(MUTED));
        return;
    }
    for line in &debug.log {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let kind = RichText::new(line.kind).font(theme::mono(11.0)).color(DISABLED);
            ui.add_sized(vec2(64.0, 16.0), Label::new(kind));
            ui.label(RichText::new(&line.text).font(theme::mono(11.0)).color(TEXT_STRONG));
        });
    }
}
