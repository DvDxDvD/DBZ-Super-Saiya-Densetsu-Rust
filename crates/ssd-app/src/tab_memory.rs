//! The Memory tab of the debug panel: "Go to" a symbol or an address, the symbols with their
//! values, an 8 x 16 hex view of the bytes around the selection, and the freezes. The state and
//! what it shows are plain code, tested without a window; the drawing at the end only reads them
//! and pushes [`PanelAction`]s.

use crate::panel::PanelAction;
use crate::theme::{
    self, ACCENT, ACCENT_FILL, BRIGHT, CONTROL, DISABLED, FIELD_LINE, GROUND, LINE_STRONG, MUTED,
    MUTED_LIGHT, SYMBOL_FILL, TEXT, VERIFY, VERIFY_FILL, VERIFY_ROW,
};
use egui::{
    Align, Align2, Button, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Ui, pos2, vec2,
};
use ssd_oracle::{Freeze, Machine, Symbols};

/// The first address of WRAM, `$7E:0000`.
pub const WRAM_BASE: u32 = 0x7E_0000;
/// Rows and columns of the hex view.
pub const HEX_ROWS: usize = 8;
pub const HEX_COLUMNS: usize = 16;
/// Bus addresses are 24 bits.
const ADDRESS_MASK: u32 = 0xFF_FFFF;

/// A freeze the panel lists: the machine holds it while it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelFreeze {
    /// The symbol it was made from, or the address.
    pub name: String,
    pub freeze: Freeze,
    pub active: bool,
}

/// What the Memory tab keeps between frames.
#[derive(Debug)]
pub struct MemoryState {
    /// The bus address the view and "Freeze selected" refer to.
    pub selection: u32,
    /// The text of the "Go to" field.
    pub query: String,
    /// The freezes made in the panel, oldest first.
    pub freezes: Vec<PanelFreeze>,
    /// Why the last freeze request was refused, in the machine's words.
    pub message: Option<String>,
    /// WRAM of the frame before the last one observed, and of the last one.
    before: Vec<u8>,
    last: Vec<u8>,
    /// The frame `last` is of (the machine's frame count).
    frame: Option<u64>,
}

impl Default for MemoryState {
    fn default() -> Self {
        Self {
            selection: WRAM_BASE,
            query: String::new(),
            freezes: Vec::new(),
            message: None,
            before: Vec::new(),
            last: Vec::new(),
            frame: None,
        }
    }
}

impl MemoryState {
    /// Takes the WRAM of `frame` (the machine's frame count) for the changed bytes. Called once
    /// per loop iteration while the tab is shown; the same frame again changes nothing.
    pub fn observe(&mut self, frame: u64, wram: &[u8]) {
        if self.frame == Some(frame) {
            return;
        }
        // After `forget` the frame is a baseline: nothing has changed since it.
        if self.frame.is_none() {
            self.last.clear();
        }
        std::mem::swap(&mut self.before, &mut self.last);
        self.last.clear();
        self.last.extend_from_slice(wram);
        self.frame = Some(frame);
    }

    /// Drops the frame observed last, while the tab is not shown: the next one observed is a
    /// baseline, not compared with the WRAM of when the tab was last shown.
    pub fn forget(&mut self) {
        self.frame = None;
    }

    /// Whether the WRAM byte at `index` (see [`wram_index`]) changed in the last observed frame.
    #[must_use]
    pub fn changed(&self, index: usize) -> bool {
        self.before.get(index).is_some() && self.before.get(index) != self.last.get(index)
    }

    /// Selects `address` and shows `query` in the field.
    pub fn select(&mut self, address: u32, query: impl Into<String>) {
        self.selection = address & ADDRESS_MASK;
        self.query = query.into();
    }

    /// Follows the field: when its text names a symbol or an address, the selection goes there.
    /// Text that names nothing leaves the selection where it is.
    pub fn follow_query(&mut self, symbols: Option<&Symbols>) {
        if let Ok(address) = parse_target(&self.query, symbols) {
            self.selection = address;
        }
    }

    /// Lists a freeze the machine accepted.
    pub fn add_freeze(&mut self, freeze: PanelFreeze) {
        self.freezes.push(freeze);
        self.message = None;
    }

    /// Turns freeze `index` on or off; `false` if there is none.
    pub fn toggle_freeze(&mut self, index: usize) -> bool {
        match self.freezes.get_mut(index) {
            Some(freeze) => {
                freeze.active = !freeze.active;
                true
            }
            None => false,
        }
    }

    /// The freezes that are on, as the machine should hold them.
    #[must_use]
    pub fn active_freezes(&self) -> Vec<Freeze> {
        self.freezes.iter().filter(|freeze| freeze.active).map(|f| f.freeze.clone()).collect()
    }
}

/// The bus address a "Go to" text names: a symbol (its first byte), `BB:OOOO`, `$BBOOOO` or
/// `BBOOOO`, the last three in hexadecimal.
///
/// # Errors
///
/// The text is none of those: the message quotes it.
pub fn parse_target(text: &str, symbols: Option<&Symbols>) -> Result<u32, String> {
    let text = text.trim();
    let named = symbols
        .and_then(|symbols| symbols.variables().iter().find(|variable| variable.name == text));
    if let Some(variable) = named {
        return Ok(variable.address);
    }
    let hex = text.strip_prefix('$').unwrap_or(text);
    let address = match hex.split_once(':') {
        Some((bank, offset)) => {
            parse_hex(bank, 2).zip(parse_hex(offset, 4)).map(|(bank, offset)| (bank << 16) | offset)
        }
        None => parse_hex(hex, 6),
    };
    address.ok_or_else(|| format!("No symbol or address \"{text}\""))
}

/// `text` as a hexadecimal number of at most `max_digits` digits.
fn parse_hex(text: &str, max_digits: usize) -> Option<u32> {
    let digits =
        !text.is_empty() && text.len() <= max_digits && text.bytes().all(|b| b.is_ascii_hexdigit());
    digits.then(|| u32::from_str_radix(text, 16).ok()).flatten()
}

/// The index in WRAM (`$7E:0000` is 0) of the bus address `address`: banks `$7E-$7F`, and the
/// low-RAM mirror `$0000-$1FFF` of banks `$00-$3F` and `$80-$BF`. `None` for any other address.
#[must_use]
pub fn wram_index(address: u32) -> Option<usize> {
    if address > ADDRESS_MASK {
        return None;
    }
    let bank = address >> 16;
    let offset = address & 0xFFFF;
    match bank {
        0x7E | 0x7F => Some((((bank & 1) << 16) | offset) as usize),
        0x00..=0x3F | 0x80..=0xBF if offset < 0x2000 => Some(offset as usize),
        _ => None,
    }
}

/// The address in WRAM (`$7E:xxxx`, as the machine keeps freezes) of a bus address in WRAM.
#[must_use]
pub fn canonical_wram(address: u32) -> Option<u32> {
    wram_index(address).map(|index| WRAM_BASE + index as u32)
}

/// A bus address as the panel writes it: `7E:1400`.
#[must_use]
pub fn address_name(address: u32) -> String {
    format!("{:02X}:{:04X}", (address >> 16) & 0xFF, address & 0xFFFF)
}

/// What "Freeze selected" freezes: the whole symbol the selection is in, else the byte at it.
/// Returns its name, first address and size in bytes.
#[must_use]
pub fn freeze_span(selection: u32, symbols: Option<&Symbols>) -> (String, u32, u32) {
    match symbols.and_then(|symbols| symbols.variable_at(selection)) {
        Some(variable) => (variable.name.clone(), variable.address, variable.size),
        None => (address_name(selection), selection, 1),
    }
}

/// The freeze "Freeze selected" makes of the selection, holding the bytes it has now.
///
/// # Errors
///
/// A byte of the span cannot be read (registers, SRAM, open bus): the message names the span.
pub fn new_freeze(
    machine: &Machine,
    selection: u32,
    symbols: Option<&Symbols>,
) -> Result<PanelFreeze, String> {
    let (name, address, size) = freeze_span(selection, symbols);
    let bytes: Option<Vec<u8>> =
        (0..size).map(|i| machine.peek(address.wrapping_add(i) & ADDRESS_MASK)).collect();
    let bytes = bytes.ok_or_else(|| format!("{name} cannot be read: nothing to freeze"))?;
    Ok(PanelFreeze { name, freeze: Freeze { address, bytes }, active: true })
}

/// A value as the panel writes it: one byte as `01`, two bytes as the word `3F20` (the low byte
/// is the first in memory), more bytes as `64 B`.
#[must_use]
pub fn value_text(bytes: &[u8]) -> String {
    match *bytes {
        [byte] => format!("{byte:02X}"),
        [low, high] => format!("{:04X}", u16::from_le_bytes([low, high])),
        _ => format!("{} B", bytes.len()),
    }
}

/// A symbol's value now: `··` where a byte cannot be read, and only its size beyond two bytes.
fn symbol_value(machine: &Machine, address: u32, size: u32) -> String {
    if size > 2 {
        return format!("{size} B");
    }
    let bytes: Option<Vec<u8>> =
        (0..size).map(|i| machine.peek(address.wrapping_add(i) & ADDRESS_MASK)).collect();
    bytes.map_or_else(|| "\u{b7}\u{b7}".to_owned(), |bytes| value_text(&bytes))
}

/// A line of the symbol list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    pub name: String,
    pub address: u32,
    pub value: String,
    /// The symbol the selection is in.
    pub selected: bool,
}

/// A byte of the hex view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cell {
    /// The byte, `None` where it cannot be read.
    pub value: Option<u8>,
    /// Part of the selected symbol.
    pub symbol: bool,
    /// Held by a freeze the machine has.
    pub frozen: bool,
    /// Changed in the last frame observed.
    pub changed: bool,
}

/// A row of the hex view: its first address and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    pub address: u32,
    pub cells: Vec<Cell>,
}

/// The symbols of `symbols` with their values now, and which one the selection is in.
#[must_use]
pub fn symbol_rows(machine: &Machine, symbols: Option<&Symbols>, selection: u32) -> Vec<SymbolRow> {
    let Some(symbols) = symbols else { return Vec::new() };
    let selected = symbols.variable_at(selection).map(|variable| variable.name.as_str());
    symbols
        .variables()
        .iter()
        .map(|variable| SymbolRow {
            name: variable.name.clone(),
            address: variable.address,
            value: symbol_value(machine, variable.address, variable.size),
            selected: selected == Some(variable.name.as_str()),
        })
        .collect()
}

/// Whether the WRAM address `address` lies in the span `(start, size)` (bus addresses).
fn in_span(address: u32, span: Option<(u32, u32)>) -> bool {
    match (canonical_wram(address), span) {
        (Some(canonical), Some((start, size))) => canonical_wram(start)
            .is_some_and(|first| first <= canonical && canonical < first + size),
        _ => false,
    }
}

/// The hex view: [`HEX_ROWS`] rows of [`HEX_COLUMNS`] bytes, from the selection's row on. `span`
/// is the selected symbol's first address and size.
#[must_use]
pub fn hex_rows(machine: &Machine, memory: &MemoryState, span: Option<(u32, u32)>) -> Vec<HexRow> {
    let columns = HEX_COLUMNS as u32;
    let first = memory.selection & !(columns - 1) & ADDRESS_MASK;
    let frozen = |canonical: u32| {
        machine.freezes().iter().any(|freeze| {
            freeze.address <= canonical && canonical < freeze.address + freeze.bytes.len() as u32
        })
    };
    (0..HEX_ROWS as u32)
        .map(|row| {
            let address = (first + row * columns) & ADDRESS_MASK;
            let cells = (0..columns)
                .map(|column| {
                    let address = (address + column) & ADDRESS_MASK;
                    Cell {
                        value: machine.peek(address),
                        symbol: in_span(address, span),
                        frozen: canonical_wram(address).is_some_and(frozen),
                        changed: wram_index(address).is_some_and(|index| memory.changed(index)),
                    }
                })
                .collect();
            HexRow { address, cells }
        })
        .collect()
}

/// What the Memory tab shows this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryView {
    pub symbols: Vec<SymbolRow>,
    pub hex: Vec<HexRow>,
    /// Why the "Go to" text names nothing (`None` when it does, or when it is empty).
    pub target_error: Option<String>,
}

/// The view of the tab, from the machine and the state.
#[must_use]
pub fn view(machine: &Machine, symbols: Option<&Symbols>, memory: &MemoryState) -> MemoryView {
    let span = symbols.and_then(|symbols| symbols.variable_at(memory.selection));
    let span = span.map(|variable| (variable.address, variable.size));
    let target_error = (!memory.query.trim().is_empty())
        .then(|| parse_target(&memory.query, symbols).err())
        .flatten();
    MemoryView {
        symbols: symbol_rows(machine, symbols, memory.selection),
        hex: hex_rows(machine, memory, span),
        target_error,
    }
}

// --- drawing ----------------------------------------------------------------------------------

/// Draws the tab: the field, the symbols, the hex view and the freezes, in one scrolling column.
pub fn show(
    ui: &mut Ui,
    view: &MemoryView,
    memory: &mut MemoryState,
    symbols: Option<&Symbols>,
    freezes_allowed: bool,
    actions: &mut Vec<PanelAction>,
) {
    ScrollArea::vertical().id_salt("memory-tab").auto_shrink([false, false]).show(ui, |ui| {
        go_to(ui, view, memory, symbols);
        symbol_list(ui, &view.symbols, memory, symbols.is_some());
        hex_view(ui, &view.hex);
        freezes(ui, memory, freezes_allowed, actions);
    });
}

fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(RichText::new(text).size(11.0).color(color));
}

fn go_to(ui: &mut Ui, view: &MemoryView, memory: &mut MemoryState, symbols: Option<&Symbols>) {
    Frame::new().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 8 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        note(ui, "Go to symbol or address", MUTED);
        ui.add_space(6.0);
        let field = Frame::new()
            .fill(GROUND)
            .stroke(Stroke::new(1.0, LINE_STRONG))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 5));
        let edit = TextEdit::singleline(&mut memory.query)
            .font(theme::mono(12.0))
            .text_color(TEXT)
            .desired_width(ui.available_width())
            .frame(field);
        if ui.add(edit).changed() {
            memory.follow_query(symbols);
        }
        if let Some(error) = &view.target_error {
            ui.add_space(4.0);
            note(ui, error, ACCENT);
        }
    });
}

fn symbol_list(ui: &mut Ui, rows: &[SymbolRow], memory: &mut MemoryState, has_symbols: bool) {
    Frame::new().inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if !has_symbols {
            note(ui, "No symbols: the symbols file could not be read.", MUTED);
        }
        for row in rows {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            let fill = if row.selected {
                ACCENT_FILL
            } else if response.hovered() {
                CONTROL
            } else {
                Color32::TRANSPARENT
            };
            let painter = ui.painter().clone();
            painter.rect_filled(rect, 5.0, fill);
            let inner = rect.shrink2(vec2(8.0, 0.0));
            let centre = inner.center().y;
            let value_right = inner.max.x;
            let address_left = value_right - 64.0 - 8.0 - 72.0;
            let font = theme::mono(12.0);
            let name =
                theme::fit_text(&painter, &row.name, &font, address_left - inner.min.x - 8.0);
            let name_color = if row.selected { ACCENT } else { TEXT };
            painter.text(pos2(inner.min.x, centre), Align2::LEFT_CENTER, name, font, name_color);
            painter.text(
                pos2(address_left, centre),
                Align2::LEFT_CENTER,
                address_name(row.address),
                theme::mono(11.0),
                MUTED,
            );
            painter.text(
                pos2(value_right, centre),
                Align2::RIGHT_CENTER,
                &row.value,
                theme::mono(12.0),
                MUTED_LIGHT,
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                memory.select(row.address, row.name.clone());
            }
        }
    });
}

/// The colours of a byte: the fill behind it and its digits.
fn cell_look(cell: Cell) -> (Option<Color32>, Color32) {
    if cell.value.is_none() {
        (None, DISABLED)
    } else if cell.frozen {
        (Some(VERIFY_FILL), VERIFY)
    } else if cell.changed {
        (cell.symbol.then_some(SYMBOL_FILL), BRIGHT)
    } else if cell.symbol {
        (Some(SYMBOL_FILL), ACCENT)
    } else {
        (None, MUTED)
    }
}

fn hex_view(ui: &mut Ui, rows: &[HexRow]) {
    Frame::new()
        .fill(GROUND)
        .stroke(Stroke::new(1.0, FIELD_LINE))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 10))
        .outer_margin(Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for row in rows {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (label, _) = ui.allocate_exact_size(vec2(52.0, 16.0), Sense::hover());
                    let painter = ui.painter().clone();
                    painter.text(
                        label.left_center(),
                        Align2::LEFT_CENTER,
                        address_name(row.address),
                        theme::mono(11.0),
                        DISABLED,
                    );
                    for cell in &row.cells {
                        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        let (fill, color) = cell_look(*cell);
                        if let Some(fill) = fill {
                            painter.rect_filled(rect, 2.0, fill);
                        }
                        let text = cell.value.map_or_else(
                            || "\u{b7}\u{b7}".to_owned(),
                            |value| format!("{value:02X}"),
                        );
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            text,
                            theme::mono(11.0),
                            color,
                        );
                    }
                });
                ui.add_space(3.0);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (color, text) in [
                    (ACCENT, "selected symbol"),
                    (VERIFY, "frozen"),
                    (BRIGHT, "changed this frame"),
                ] {
                    ui.label(RichText::new("\u{25a0}").size(11.0).color(color));
                    note(ui, text, MUTED);
                }
            });
        });
}

fn freezes(
    ui: &mut Ui,
    memory: &MemoryState,
    freezes_allowed: bool,
    actions: &mut Vec<PanelAction>,
) {
    Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Freezes").size(12.0).color(MUTED_LIGHT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = Button::new(RichText::new("Freeze selected").size(12.0))
                    .min_size(vec2(0.0, 26.0));
                if ui.add_enabled(freezes_allowed, button).clicked() {
                    actions.push(PanelAction::FreezeSelected);
                }
            });
        });
        if !freezes_allowed {
            note(ui, "Freezes are off while a replay records or plays.", ACCENT);
        }
        if let Some(message) = &memory.message {
            note(ui, message, ACCENT);
        }
        if memory.freezes.is_empty() {
            note(ui, "No freezes.", MUTED);
        }
        for (index, freeze) in memory.freezes.iter().enumerate() {
            let fill = if freeze.active { VERIFY_ROW } else { Color32::TRANSPARENT };
            Frame::new()
                .fill(fill)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let mut on = freeze.active;
                        let check = egui::Checkbox::without_text(&mut on);
                        if ui.add_enabled(freezes_allowed, check).changed() {
                            actions.push(PanelAction::ToggleFreeze(index));
                        }
                        ui.label(RichText::new(&freeze.name).font(theme::mono(12.0)).color(TEXT));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let value = format!("= {}", value_text(&freeze.freeze.bytes));
                            ui.label(RichText::new(value).font(theme::mono(12.0)).color(VERIFY));
                        });
                    });
                });
        }
        ui.add_space(6.0);
        note(
            ui,
            "Named cheats (health, ki, cards) arrive with the M2 catalogue. Freezes are refused \
             while recording a replay.",
            MUTED,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{SharedStore, sram_counter_machine};
    use std::path::Path;

    /// Two bytes, one byte and two 64-byte records, as the repository's symbols name them.
    fn symbols() -> Symbols {
        let source = "\
[[variable]]
name = \"rng_seed_a\"
address = 0x7E002F
size = 2

[[variable]]
name = \"active_side\"
address = 0x7E14C2

[[variable]]
name = \"side_record_a\"
address = 0x7E1400
size = 0x40

[[variable]]
name = \"side_record_b\"
address = 0x7E1440
size = 0x40
";
        Symbols::parse(source, Path::new("symbols.toml")).unwrap()
    }

    fn machine() -> (Machine, SharedStore) {
        sram_counter_machine()
    }

    #[test]
    fn a_target_is_a_symbol_or_an_address_in_hex() {
        let symbols = symbols();
        let parse = |text| parse_target(text, Some(&symbols));
        assert_eq!(parse("side_record_b"), Ok(0x7E_1440));
        assert_eq!(parse("7E:1400"), Ok(0x7E_1400));
        assert_eq!(parse("7e:14c2"), Ok(0x7E_14C2));
        assert_eq!(parse("$7E1400"), Ok(0x7E_1400));
        assert_eq!(parse("  $7E:1403 "), Ok(0x7E_1403), "spaces around the text");
        assert_eq!(parse("1400"), Ok(0x00_1400), "six or fewer hex digits");
        for bad in ["", "$", "7E:ZZ", "7E:14001", "1234567", "zz", "side_record_c", "7E:"] {
            let error = parse(bad).unwrap_err();
            assert!(error.contains("No symbol or address"), "{bad}: {error}");
        }
        assert!(parse_target("active_side", None).is_err(), "no symbols, no names");
    }

    #[test]
    fn wram_is_the_banks_and_the_low_ram_mirror() {
        assert_eq!(wram_index(0x7E_0000), Some(0));
        assert_eq!(wram_index(0x7F_FFFF), Some(0x1_FFFF));
        assert_eq!(wram_index(0x00_1FFF), Some(0x1FFF));
        assert_eq!(wram_index(0x80_0010), Some(0x10));
        assert_eq!(wram_index(0x3F_0010), Some(0x10));
        for outside in [0x00_2000, 0x01_2000, 0x70_0000, 0x7D_0000, 0x40_0000, 0x0100_0000] {
            assert_eq!(wram_index(outside), None, "${outside:06X}");
        }
        assert_eq!(canonical_wram(0x00_1400), Some(0x7E_1400));
        assert_eq!(canonical_wram(0x80_1400), Some(0x7E_1400));
    }

    #[test]
    fn values_are_hex_for_a_byte_or_a_word_and_a_size_beyond() {
        assert_eq!(value_text(&[0x01]), "01");
        assert_eq!(value_text(&[0x20, 0x3F]), "3F20", "the low byte is first in memory");
        assert_eq!(value_text(&[0; 64]), "64 B");
        assert_eq!(value_text(&[1, 2, 3]), "3 B");
    }

    #[test]
    fn the_symbols_show_their_values_and_the_selected_one() {
        let symbols = symbols();
        let (mut m, _store) = machine();
        m.bus_write(0x7E_002F, 0x20);
        m.bus_write(0x7E_0030, 0x3F);
        m.bus_write(0x7E_14C2, 0x01);
        let rows = symbol_rows(&m, Some(&symbols), 0x7E_1403);
        let row = |name: &str| rows.iter().find(|row| row.name == name).unwrap();
        assert_eq!(row("rng_seed_a").value, "3F20");
        assert_eq!(row("active_side").value, "01");
        assert_eq!(row("side_record_a").value, "64 B");
        assert!(row("side_record_a").selected, "the selection is in the record");
        assert_eq!(rows.iter().filter(|row| row.selected).count(), 1);
        assert_eq!(symbol_rows(&m, None, 0x7E_1403), Vec::new(), "no file, no list");
    }

    #[test]
    fn the_hex_view_starts_at_the_selections_row_and_marks_the_symbol_frozen_and_changed() {
        let symbols = symbols();
        let (mut m, _store) = machine();
        m.bus_write(0x7E_1403, 0xAB);
        let mut memory = MemoryState::default();
        memory.select(0x7E_1403, "");
        memory.observe(1, m.wram());
        m.bus_write(0x7E_1403, 0xCD);
        memory.observe(2, m.wram());
        let span = symbols.variable_at(0x7E_1403).map(|v| (v.address, v.size));
        m.set_freezes(vec![Freeze { address: 0x7E_1440, bytes: vec![0x11] }]).unwrap();

        let rows = hex_rows(&m, &memory, span);
        assert_eq!(rows.len(), HEX_ROWS);
        assert_eq!(rows[0].address, 0x7E_1400, "the row of the selection");
        let cell = |row: usize, column: usize| rows[row].cells[column];
        assert_eq!(cell(0, 3).value, Some(0xCD));
        assert!(cell(0, 3).symbol && cell(0, 3).changed && !cell(0, 3).frozen);
        assert!(cell(0, 0).symbol && !cell(0, 0).changed, "in the symbol, not changed");
        assert!(!cell(4, 0).symbol, "the next record is not the selected symbol");
        assert!(cell(4, 0).frozen, "a freeze holds the byte at $7E:1440");
        assert!(!cell(4, 1).frozen);
        assert!(!cell(0, 4).changed, "only the byte that changed is marked");
    }

    #[test]
    fn a_byte_nothing_reads_has_no_value_and_no_colour_of_its_own() {
        let (m, _store) = machine();
        let memory = MemoryState { selection: 0x00_2100, ..MemoryState::default() };
        let rows = hex_rows(&m, &memory, None);
        assert_eq!(rows[0].cells[0].value, None, "a register");
        assert_eq!(cell_look(rows[0].cells[0]), (None, DISABLED));
        assert_eq!(rows[0].address, 0x00_2100, "the row starts at the selection's row");
    }

    #[test]
    fn observing_a_frame_marks_what_changed_since_the_one_before() {
        let mut memory = MemoryState::default();
        let mut wram = vec![0_u8; 0x2_0000];
        memory.observe(1, &wram);
        assert!(!memory.changed(5), "nothing before the first frame");
        wram[5] = 9;
        memory.observe(2, &wram);
        assert!(memory.changed(5));
        memory.observe(2, &wram);
        assert!(memory.changed(5), "the same frame changes nothing");
        memory.observe(3, &wram);
        assert!(!memory.changed(5));
    }

    #[test]
    fn the_first_frame_after_the_tab_was_hidden_is_a_baseline() {
        let mut memory = MemoryState::default();
        let mut wram = vec![0_u8; 0x2_0000];
        memory.observe(1, &wram);
        wram[5] = 9;
        memory.observe(2, &wram);
        memory.forget();
        // Byte 5 changed while the tab was hidden: that is not a change of the frame shown.
        wram[6] = 4;
        memory.observe(7, &wram);
        assert!(!memory.changed(5) && !memory.changed(6), "a baseline marks nothing");
        wram[6] = 5;
        memory.observe(8, &wram);
        assert!(memory.changed(6), "the next frame is compared with the baseline");
    }

    #[test]
    fn freeze_selected_takes_the_symbol_or_the_byte_and_its_current_bytes() {
        let symbols = symbols();
        let (mut m, _store) = machine();
        m.bus_write(0x7E_14C2, 0x01);
        assert_eq!(
            freeze_span(0x7E_1403, Some(&symbols)),
            ("side_record_a".to_owned(), 0x7E_1400, 0x40)
        );
        assert_eq!(freeze_span(0x7E_1403, None), ("7E:1403".to_owned(), 0x7E_1403, 1));
        let freeze = new_freeze(&m, 0x7E_14C2, Some(&symbols)).unwrap();
        assert_eq!(freeze.name, "active_side");
        assert_eq!(freeze.freeze, Freeze { address: 0x7E_14C2, bytes: vec![0x01] });
        assert!(freeze.active);
        let error = new_freeze(&m, 0x00_2100, None).unwrap_err();
        assert!(error.contains("nothing to freeze"), "{error}");
    }

    #[test]
    fn a_panel_freeze_is_listed_and_toggles_and_only_the_active_ones_go_to_the_machine() {
        let mut memory = MemoryState::default();
        let freeze = |name: &str, active| PanelFreeze {
            name: name.to_owned(),
            freeze: Freeze { address: 0x7E_0010, bytes: vec![1] },
            active,
        };
        memory.add_freeze(freeze("a", true));
        memory.add_freeze(freeze("b", false));
        assert_eq!(memory.active_freezes(), vec![freeze("a", true).freeze]);
        assert!(memory.toggle_freeze(1));
        assert_eq!(memory.active_freezes().len(), 2);
        assert!(!memory.toggle_freeze(2), "there is no third freeze");
    }

    #[test]
    fn the_field_moves_the_selection_when_it_names_something_and_says_when_not() {
        let symbols = symbols();
        let (m, _store) = machine();
        let mut memory =
            MemoryState { query: "side_record_b".to_owned(), ..MemoryState::default() };
        memory.follow_query(Some(&symbols));
        assert_eq!(memory.selection, 0x7E_1440);
        memory.query = "7E:14".to_owned();
        memory.follow_query(Some(&symbols));
        assert_eq!(memory.selection, 0x7E_0014, "an address being typed is followed");
        memory.query = "not here".to_owned();
        memory.follow_query(Some(&symbols));
        assert_eq!(memory.selection, 0x7E_0014, "a text that names nothing keeps the selection");
        assert_eq!(
            view(&m, Some(&symbols), &memory).target_error.as_deref(),
            Some("No symbol or address \"not here\"")
        );
        memory.query.clear();
        assert_eq!(
            view(&m, Some(&symbols), &memory).target_error,
            None,
            "an empty field is no error"
        );
    }
}
