//! The HD screens: the field's command window with its help, the Menu list, and the item list with
//! its detail. Each builds a [`DrawList`] from a [`MenuView`] at the positions of the approved design.
//! The positions are the design's; the text they hold is the game's names and our own English help.

use crate::cheats::Row;
use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{
    BAR_HEIGHT, COMMAND_ROW, CORNER_PORTRAIT, CURSOR_HEIGHT, CURSOR_WIDTH, FRAME, HEIGHT,
    HELP_LINE, ITEM_PORTRAIT, ITEM_ROW, LARGE_TEXT, LINE, MARGIN, PADDING, PORTRAIT_FRAME,
    SMALL_TEXT, WIDTH, fit, palette, text_width,
};
use crate::nav::{Entry, MENU_ROWS, command_rows};
use crate::view::{BagView, CheatsView, Member, MenuView, Screen, Selected, Sheet, Tab};

/// The items shown on one page of the item list.
pub const PAGE_SIZE: usize = 8;
/// The capacity the item list shows in its header: the original list's 24 entries.
pub const CAPACITY: usize = 24;
/// The space a name keeps from the text beside it (the count of a stack, the figures of a party member).
const NAME_GAP: f32 = 12.0;
/// The figures of a party member on the Use sheet: small enough that a name of the party fits beside them.
const FIGURE_TEXT: f32 = 14.0;
/// Distance of the corner pictures of the field menu from the sides of the space.
const CORNER_INSET: f32 = 32.0;
/// Distance of the corner pictures of the field menu from the top.
const CORNER_TOP: f32 = 28.0;

/// The rows of the command window of `view`: the field's, a town's or the flying ones.
#[must_use]
pub fn commands(view: &MenuView) -> &'static [Entry] {
    command_rows(view.flying, view.town)
}

/// The draw list of the HD screen of `view`. The cheats screen has its own view: see [`cheats_screen`].
#[must_use]
pub fn scene(view: &MenuView) -> DrawList {
    match view.screen {
        Screen::Command => command(view),
        Screen::Menu => menu_list(view),
        Screen::Items => items(view),
        Screen::Cheats => DrawList::new(),
    }
}

/// The field's command window, with the party's corners and the money.
#[must_use]
pub fn command(view: &MenuView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    party(&mut list, view);
    zeni(&mut list, view.zeni);
    let rows = commands(view);
    list_window(&mut list, rows, view.cursor);
    help_window(&mut list, rows, view.cursor);
    list
}

/// The Menu list: the party's corners and the money as on the field, then the list of Cards, Status,
/// Order, Text and Save, with Cheats and Options greyed out.
#[must_use]
pub fn menu_list(view: &MenuView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    party(&mut list, view);
    zeni(&mut list, view.zeni);
    list_window(&mut list, &MENU_ROWS, view.cursor);
    help_window(&mut list, &MENU_ROWS, view.cursor);
    list
}

/// The bag: its stacks in two columns, the tabs, the sheet over the list and the object under the
/// cursor with its picture, price and help. Without a bag, the original's entries.
#[must_use]
pub fn items(view: &MenuView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    match &view.bag {
        Some(bag) => {
            bag_window(&mut list, view, bag);
            zeni_window(&mut list, view.zeni);
            detail(&mut list, view.selected.as_ref());
            if let Some(sheet) = sheet_window(bag, &view.party) {
                sheet_draw(&mut list, &sheet);
            }
        }
        None => {
            bag_entries(&mut list, view);
            zeni_window(&mut list, view.zeni);
            detail(&mut list, view.selected.as_ref());
        }
    }
    list
}

/// The ground of the screens: the field as it was when the menu opened, which the app paints.
fn backdrop(list: &mut DrawList) {
    list.push(Op::Backdrop);
}

/// The party's corner pictures with their HP and KI bars, in party order (two corners).
fn party(list: &mut DrawList, view: &MenuView) {
    for (member, &x) in view.party.iter().take(2).zip(CORNERS.iter()) {
        let frame = Rect::new(x, CORNER_TOP, CORNER_PORTRAIT, CORNER_PORTRAIT);
        list.push(Op::Portrait { rect: frame, object: member.face });
        let hp = frame.bottom() + 14.0;
        let ki = hp + 20.0;
        gauge_row(
            list,
            GaugeRow { x, y: hp, width: frame.w, label: "HP", fill: palette::HP },
            (member.hp, member.max_hp),
        );
        gauge_row(
            list,
            GaugeRow { x, y: ki, width: frame.w, label: "KI", fill: palette::KI },
            (member.ki, member.max_ki),
        );
    }
}

/// Where the corner pictures of the field menu sit: the left one and the right one, 28 px down.
const CORNERS: [f32; 2] = [CORNER_INSET, WIDTH - CORNER_INSET - CORNER_PORTRAIT];

/// One row of a member's gauges: where it starts, how wide the column is, its label and its fill.
struct GaugeRow {
    x: f32,
    y: f32,
    width: f32,
    label: &'static str,
    fill: Color,
}

/// A member's HP or KI under the corner picture: its label, in white with a black shadow, and its
/// bar. `amount` is the value and the maximum.
fn gauge_row(list: &mut DrawList, row: GaugeRow, amount: (u16, u16)) {
    let GaugeRow { x, y, width, label, fill } = row;
    list.text(x + 2.0, y + 2.0, label, SMALL_TEXT, palette::BLACK, Align::Left);
    list.text(x, y, label, SMALL_TEXT, palette::WHITE, Align::Left);
    let (value, max) = amount;
    let fraction = if max == 0 { 0.0 } else { (f32::from(value) / f32::from(max)).min(1.0) };
    let bar = Rect::new(x + 28.0, y - BAR_HEIGHT / 2.0, width - 28.0, BAR_HEIGHT);
    list.push(Op::Bar { rect: bar, fraction, color: fill });
}

/// The money, centred at the top of the field menu.
fn zeni(list: &mut DrawList, zeni: u16) {
    let window = Rect::new((WIDTH - 240.0) / 2.0, 36.0, 240.0, 72.0);
    list.push(Op::Window { rect: window });
    let text = format!("Zeni {zeni:03}P");
    list.text(
        window.x + window.w / 2.0,
        window.middle_y(),
        &text,
        LARGE_TEXT,
        palette::BLACK,
        Align::Centre,
    );
}

/// A list window at the bottom left, one row per entry, with the cursor on the row under it. An
/// entry that is not enabled is greyed out and says it is coming soon.
fn list_window(list: &mut DrawList, rows: &[Entry], cursor: u8) {
    let height = 2.0 * FRAME + 2.0 * 14.0 + COMMAND_ROW * rows.len() as f32;
    // A greyed entry carries its "coming soon" note after its label: the window makes room for it.
    let width = if rows.iter().any(|entry| !entry.enabled()) { 360.0 } else { 220.0 };
    let window = Rect::new(MARGIN, HEIGHT - MARGIN - height, width, height);
    list.push(Op::Window { rect: window });
    let first = window.y + FRAME + 14.0;
    for (row, entry) in rows.iter().enumerate() {
        let middle = first + COMMAND_ROW * row as f32 + COMMAND_ROW / 2.0;
        if usize::from(cursor) == row {
            list.push(Op::Cursor { x: window.x + FRAME + 14.0, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if entry.enabled() { palette::BLACK } else { palette::GREY };
        list.text(
            window.x + FRAME + 14.0 + CURSOR_WIDTH + 10.0,
            middle,
            entry.label(),
            LARGE_TEXT,
            colour,
            Align::Left,
        );
        if !entry.enabled() {
            list.text(
                window.right() - FRAME - 14.0,
                middle,
                "coming soon",
                SMALL_TEXT,
                palette::GREY,
                Align::Right,
            );
        }
    }
}

/// The help window at the bottom right: the help of the entry under the cursor.
fn help_window(list: &mut DrawList, rows: &[Entry], cursor: u8) {
    let help = rows.get(usize::from(cursor)).map_or("", |entry| entry.help());
    let height = help_height(1);
    let window = Rect::new(WIDTH - MARGIN - 560.0, HEIGHT - MARGIN - height, 560.0, height);
    list.push(Op::Window { rect: window });
    list.text(
        window.x + FRAME + 22.0,
        window.y + FRAME + PADDING + HELP_LINE / 2.0,
        help,
        LARGE_TEXT,
        palette::BLACK,
        Align::Left,
    );
    hints(list, window);
}

/// The height of a help window with `lines` lines of text.
#[must_use]
fn help_height(lines: usize) -> f32 {
    2.0 * FRAME + 2.0 * PADDING + HELP_LINE * lines.max(1) as f32 + 8.0 + SMALL_TEXT
}

/// The key hints along the bottom of a help window.
fn hints(list: &mut DrawList, window: Rect) {
    let y = window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0;
    for (i, hint) in ["A  OK", "B  Back", "Start  Close"].iter().enumerate() {
        list.text(
            window.x + FRAME + 22.0 + 120.0 * i as f32,
            y,
            hint,
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Left,
        );
    }
}

/// The bag window: the header with the slots in use, the tabs, the stacks of the page in two
/// columns with their counts, and the page with the notice at its foot.
fn bag_window(list: &mut DrawList, view: &MenuView, bag: &BagView) {
    let window = Rect::new(MARGIN, 36.0, 640.0, 570.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    let right = window.right() - FRAME - 22.0;
    let header = window.y + FRAME + 16.0 + 16.0;
    list.text(left, header, "Items", LARGE_TEXT, palette::BLACK, Align::Left);
    let count = format!("{:02}/{:02}", bag.used, bag.slots);
    list.text(right, header, &count, SMALL_TEXT, palette::DARK_RED, Align::Right);
    // The tabs: the current one is marked with an orange bar.
    let tabs_y = header + 34.0;
    let mut tab_x = left;
    for tab in [Tab::All, Tab::Allies, Tab::Tools] {
        let current = tab == bag.tab;
        let label = tab.label();
        let width = 40.0 + 16.0 * label.len() as f32;
        let colour = if current { palette::DARK_RED } else { palette::GREY };
        list.text(tab_x + 8.0, tabs_y, label, SMALL_TEXT, colour, Align::Left);
        if current {
            list.fill(Rect::new(tab_x, tabs_y + 12.0, width - 16.0, LINE * 2.0), palette::ORANGE);
        }
        tab_x += width;
    }
    let rule_y = tabs_y + 20.0;
    list.fill(Rect::new(left, rule_y, right - left, LINE), palette::DARK_RED);

    let column_width = (right - left - 30.0) / 2.0;
    let first_row = rule_y + 14.0 + ITEM_ROW / 2.0;
    let page = usize::from(view.page);
    for index in 0..PAGE_SIZE {
        let column = index / 4;
        let row = index % 4;
        let x = left + column as f32 * (column_width + 30.0);
        let middle = first_row + row as f32 * ITEM_ROW;
        let Some(stack) = bag.stacks.get(page * PAGE_SIZE + index) else { continue };
        if usize::from(view.cursor) == index {
            list.push(Op::Cursor { x, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if stack.usable { palette::BLACK } else { palette::GREY };
        let units = format!("\u{d7}{:02}", stack.count);
        // The name stops before the count, which is right-aligned at the end of the column.
        let room = column_width - 26.0 - text_width(&units, SMALL_TEXT) - NAME_GAP;
        let name = fit(&stack.name, LARGE_TEXT, room);
        list.text(x + 26.0, middle, &name, LARGE_TEXT, colour, Align::Left);
        list.text(x + column_width, middle, &units, SMALL_TEXT, colour, Align::Right);
    }
    let pages = bag.stacks.len().div_ceil(PAGE_SIZE).max(1);
    let footer_y = window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0;
    let footer = format!("Page {}/{pages}", page + 1);
    list.text(right, footer_y, &footer, SMALL_TEXT, palette::DARK_RED, Align::Right);
    match (bag.notice, bag.full) {
        (Some(notice), _) => {
            list.text(left, footer_y, notice, SMALL_TEXT, palette::DARK_RED, Align::Left);
        }
        (None, true) => list.text(
            left,
            footer_y,
            "Bag full: nothing more can be taken.",
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Left,
        ),
        (None, false) => {}
    }
}

/// The original's entries, when there is no bag yet: one row each, the same window.
fn bag_entries(list: &mut DrawList, view: &MenuView) {
    let window = Rect::new(MARGIN, 36.0, 640.0, 570.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    let right = window.right() - FRAME - 22.0;
    let header = window.y + FRAME + 16.0 + 16.0;
    list.text(left, header, "Items", LARGE_TEXT, palette::BLACK, Align::Left);
    let count = format!("{:02}/{CAPACITY}", view.items.len());
    list.text(right, header, &count, SMALL_TEXT, palette::DARK_RED, Align::Right);
    list.fill(Rect::new(left, header + 26.0, right - left, LINE), palette::DARK_RED);
    let column_width = (right - left - 30.0) / 2.0;
    let first_row = header + 26.0 + 14.0 + ITEM_ROW / 2.0;
    let page = usize::from(view.page);
    for index in 0..PAGE_SIZE {
        let column = index / 4;
        let row = index % 4;
        let x = left + column as f32 * (column_width + 30.0);
        let middle = first_row + row as f32 * ITEM_ROW;
        let Some(slot) = view.items.get(page * PAGE_SIZE + index) else { continue };
        if usize::from(view.cursor) == index {
            list.push(Op::Cursor { x, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let name = fit(&slot.name, LARGE_TEXT, column_width - 26.0 - NAME_GAP);
        list.text(x + 26.0, middle, &name, LARGE_TEXT, palette::BLACK, Align::Left);
    }
    let pages = view.items.len().div_ceil(PAGE_SIZE).max(1);
    let footer = format!("Page {}/{pages}", page + 1);
    let footer_y = window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0;
    list.text(right, footer_y, &footer, SMALL_TEXT, palette::DARK_RED, Align::Right);
}

/// A sheet over the list: its frame, its title and its rows, with the cursor on one of them.
struct SheetWindow {
    rect: Rect,
    title: String,
    /// Each row: its label, its figures (the use-on list's LV, HP, KI, BP) and whether it is enabled.
    rows: Vec<(String, Option<String>, bool)>,
    cursor: Option<usize>,
    /// The units chosen, on the quantity sheet: drawn as ◀ ×NN ▶ with shapes for the arrows.
    quantity: Option<u16>,
    /// Distance between two rows: [`SHEET_ROW`], or less when the rows would run past the space.
    pitch: f32,
}

/// Top of the sheets, under the detail's picture and help.
const SHEET_TOP: f32 = 300.0;
/// Distance between two rows of a sheet.
const SHEET_ROW: f32 = 40.0;

/// The sheet open on the bag, if one is: Use or Drop, the party's use-on list, the quantity, or
/// Drop's Yes and No. It takes the place of the detail's help, on the right.
fn sheet_window(bag: &BagView, party: &[Member]) -> Option<SheetWindow> {
    let x = 702.0;
    let width = 538.0;
    let (title, rows, cursor) = match bag.sheet {
        Sheet::Browse => return None,
        Sheet::Choose => (
            "Use or drop?".to_owned(),
            vec![
                ("Use".to_owned(), None, bag.usable_here),
                ("Drop".to_owned(), None, bag.droppable_here),
            ],
            Some(usize::from(bag.choice)),
        ),
        Sheet::Members => (
            "Use on whom?".to_owned(),
            party
                .iter()
                .map(|member| {
                    let figures = format!(
                        "LV {:>2} HP {:>3}/{:<3} KI {:>3}/{:<3} BP {:>3}",
                        member.level, member.hp, member.max_hp, member.ki, member.max_ki, member.bp
                    );
                    (member.name.clone(), Some(figures), true)
                })
                .collect(),
            Some(usize::from(bag.member)),
        ),
        Sheet::Quantity => ("How many?".to_owned(), Vec::new(), None),
        Sheet::Confirm => (
            format!("Drop \u{d7}{:02}?", bag.units),
            vec![("Yes".to_owned(), None, true), ("No".to_owned(), None, true)],
            Some(usize::from(bag.choice)),
        ),
    };
    // Above the rows: the frame, the padding, the title and its gap.
    let head = 2.0 * FRAME + 2.0 * PADDING + LARGE_TEXT + 18.0;
    // The rows are 40 px apart; a party too long for the space below the detail takes closer rows,
    // so that the nine records a party can have still end above the margin.
    let room = HEIGHT - MARGIN - SHEET_TOP - head;
    let pitch = (room / rows.len().max(1) as f32).min(SHEET_ROW);
    let height = (head + pitch * rows.len() as f32).max(150.0);
    let quantity = (bag.sheet == Sheet::Quantity).then_some(bag.units);
    Some(SheetWindow {
        rect: Rect::new(x, SHEET_TOP, width, height),
        title,
        rows,
        cursor,
        quantity,
        pitch,
    })
}

/// Paints a sheet: its frame, its title, and its rows with the cursor on the chosen one.
fn sheet_draw(list: &mut DrawList, window: &SheetWindow) {
    list.push(Op::Window { rect: window.rect });
    let left = window.rect.x + FRAME + 22.0;
    let top = window.rect.y + FRAME + PADDING;
    list.text(left, top + LARGE_TEXT / 2.0, &window.title, LARGE_TEXT, palette::BLACK, Align::Left);
    if let Some(units) = window.quantity {
        // ◀ ×NN ▶, centred under the title: the font has no triangles, so they are shapes.
        let middle = top + LARGE_TEXT + 18.0 + 20.0;
        let centre = window.rect.x + window.rect.w / 2.0;
        let text = format!("\u{d7}{units:02}");
        list.text(centre, middle, &text, LARGE_TEXT, palette::BLACK, Align::Centre);
        let reach = 60.0;
        let y = middle - CURSOR_HEIGHT / 2.0;
        list.push(Op::Arrow { x: centre - reach - CURSOR_WIDTH, y, left: true });
        list.push(Op::Arrow { x: centre + reach, y, left: false });
    }
    for (row, (label, figures, enabled)) in window.rows.iter().enumerate() {
        let middle = top + LARGE_TEXT + 18.0 + window.pitch * row as f32 + 20.0;
        if window.cursor == Some(row) {
            list.push(Op::Cursor { x: left - 4.0, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if *enabled { palette::BLACK } else { palette::GREY };
        let name_left = left + CURSOR_WIDTH + 10.0;
        let right = window.rect.right() - FRAME - 22.0;
        // The figures take the end of the row, and the name stops before them.
        let figures_width = figures.as_deref().map_or(0.0, |text| text_width(text, FIGURE_TEXT));
        let name = fit(label, LARGE_TEXT, right - figures_width - NAME_GAP - name_left);
        list.text(name_left, middle, &name, LARGE_TEXT, colour, Align::Left);
        if let Some(figures) = figures {
            list.text(right, middle, figures, FIGURE_TEXT, palette::DARK_RED, Align::Right);
        }
    }
}

/// The money at the bottom left, under the bag.
fn zeni_window(list: &mut DrawList, zeni: u16) {
    let window = Rect::new(MARGIN, 628.0, 640.0, 56.0);
    list.push(Op::Window { rect: window });
    list.text(
        window.x + FRAME + 20.0,
        window.middle_y(),
        "Zeni",
        LARGE_TEXT,
        palette::BLACK,
        Align::Left,
    );
    let amount = format!("{zeni:03}P");
    list.text(
        window.right() - FRAME - 20.0,
        window.middle_y(),
        &amount,
        LARGE_TEXT,
        palette::BLACK,
        Align::Right,
    );
}

/// The object under the cursor: its picture and price box on the left, its help on the right.
fn detail(list: &mut DrawList, selected: Option<&Selected>) {
    let x = WIDTH - MARGIN - 538.0;
    let picture = Rect::new(x, 36.0, ITEM_PORTRAIT, ITEM_PORTRAIT);
    list.push(Op::Portrait { rect: picture, object: selected.map(|s| s.object) });
    let price = Rect::new(x, picture.bottom() + 4.0, ITEM_PORTRAIT, 46.0);
    list.fill(price, palette::PRICE);
    list.push(Op::Outline { rect: price, width: PORTRAIT_FRAME, color: palette::BLACK });
    // An object that is not sold shows no figure and no unit, as in the original (its box holds
    // a small kanji there instead); an empty list shows an empty box
    if let Some(figure) = selected.and_then(|s| s.price) {
        // The figure ends a 4 px gap before the unit, which ends 10 px from the box's edge.
        let unit_left = price.right() - 10.0 - text_width("Pt", SMALL_TEXT);
        list.text(
            unit_left - 4.0,
            price.middle_y(),
            &figure.to_string(),
            LARGE_TEXT,
            palette::PRICE_DIGITS,
            Align::Right,
        );
        list.text(
            price.right() - 10.0,
            price.middle_y(),
            "Pt",
            SMALL_TEXT,
            palette::PRICE_DIGITS,
            Align::Right,
        );
    }

    let Some(selected) = selected else { return };
    let window =
        Rect::new(picture.right() + 22.0, 36.0, WIDTH - MARGIN - (picture.right() + 22.0), 240.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    // Every line of the panel stops at its inner edge.
    let room = window.right() - FRAME - 22.0 - left;
    let top = window.y + FRAME + PADDING + LARGE_TEXT / 2.0;
    let name = fit(&selected.name, LARGE_TEXT, room);
    list.text(left, top, &name, LARGE_TEXT, palette::BLACK, Align::Left);
    for (line, text) in selected.help.iter().take(3).enumerate() {
        let text = fit(text, LARGE_TEXT, room);
        list.text(
            left,
            top + HELP_LINE * (line + 1) as f32,
            &text,
            LARGE_TEXT,
            palette::BLACK,
            Align::Left,
        );
    }
    let usable = match (selected.field, selected.battle) {
        (true, true) => "Usable: on the map and in battle",
        (true, false) => "Usable: on the map",
        (false, true) => "Usable: in battle",
        (false, false) => "Not usable from the menu",
    };
    list.text(
        left,
        window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0,
        &fit(usable, SMALL_TEXT, room),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
}

/// The grid of the Cheats screen, in logical pixels, as the approved design sets it: two panels of
/// [`CHEATS_COLUMN`] px and the preview column of [`CHEATS_SIDE`] px, [`CHEATS_GAP`] px apart, from
/// [`CHEATS_EDGE`] px below the top of the space to the help window at its foot.
const CHEATS_COLUMN: f32 = 443.0;
const CHEATS_SIDE: f32 = 270.0;
const CHEATS_GAP: f32 = 22.0;
const CHEATS_EDGE: f32 = 32.0;
/// Height of the help window at the foot.
const CHEATS_HELP: f32 = 58.0;
/// Height of the window of the next card: its title, the card and its caption.
const CHEATS_CARD: f32 = 286.0;
/// The panels' padding: 12 px above their first group, 18 px at the sides.
const PANEL_TOP: f32 = 12.0;
const PANEL_SIDE: f32 = 18.0;
/// A group's title: 8 px above it, 15 px high, 2 px below it.
const GROUP_TITLE_TOP: f32 = 8.0;
const GROUP_TITLE_HEIGHT: f32 = 18.0;
const GROUP_TITLE_BELOW: f32 = 2.0;
/// The text sizes of the screen, as the design sets them.
const GROUP_TEXT: f32 = 15.0;
const ROW_TEXT: f32 = 18.0;
const CAPTION_TEXT: f32 = 14.0;
/// A row of a panel is at least 44 px high; the rows and the titles are 2 px apart.
const ROW_HEIGHT: f32 = 44.0;
const LINE_GAP: f32 = 2.0;
/// The choice buttons are 40 px high and at least 40 px wide (their text has 4 px of padding each side); a
/// cycler's arrows sit in 44 px boxes, 4 px from the value.
const CHOICE_HEIGHT: f32 = 40.0;
const CHOICE_MIN: f32 = 40.0;
const CHOICE_PADDING: f32 = 8.0;
const ARROW_BOX: f32 = 44.0;
const CYCLER_GAP: f32 = 4.0;
/// The value of a cycler is at least 120 px wide.
const CYCLER_VALUE: f32 = 120.0;
/// The card of the preview: 124 x 184 px with a 6 px white border; its balls are 42 px across.
const CARD_WIDTH: f32 = 124.0;
const CARD_HEIGHT: f32 = 184.0;
const CARD_BORDER: f32 = 6.0;
const BALL: f32 = 42.0;
/// The window's inner padding: 16 px at the sides, 14 px above and below.
const WINDOW_SIDE: f32 = 16.0;
const WINDOW_TOP: f32 = 14.0;
/// The characters a line of the captions and of the On list's empty text holds: a line of 14 px text is about
/// 15 characters in the side column, 16 px text about 14.
const CAPTION_CHARS: usize = 15;
const EMPTY_CHARS: usize = 14;
/// The Reset all button: 44 px high, at the foot of the On window.
const RESET_HEIGHT: f32 = 44.0;

/// The Cheats screen: the choices of the run in two panels (Battle and Cards on the left, Resources and
/// Encounters in the middle), the preview of the next card, the On list with Reset all on the right, and the
/// help of the row under the cursor at the foot. The choice that is on is orange with white text; the row
/// under the cursor has the orange cursor. Everything lies inside the space.
#[must_use]
pub fn cheats_screen(view: &CheatsView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    let bottom = HEIGHT - CHEATS_EDGE - CHEATS_HELP - CHEATS_GAP;
    let height = bottom - CHEATS_EDGE;
    let left = Rect::new(MARGIN, CHEATS_EDGE, CHEATS_COLUMN, height);
    let middle = Rect::new(left.right() + CHEATS_GAP, CHEATS_EDGE, CHEATS_COLUMN, height);
    let side_x = middle.right() + CHEATS_GAP;
    let card = Rect::new(side_x, CHEATS_EDGE, CHEATS_SIDE, CHEATS_CARD);
    let on = Rect::new(
        side_x,
        card.bottom() + CHEATS_GAP,
        CHEATS_SIDE,
        bottom - card.bottom() - CHEATS_GAP,
    );
    cheats_panel(
        &mut list,
        view,
        left,
        &[&[Row::Damage, Row::Defence, Row::Exp], &[Row::ZCards, Row::ZFor]],
    );
    cheats_panel(
        &mut list,
        view,
        middle,
        &[
            &[Row::MaxZeni, Row::EndlessItems],
            &[Row::RandomBattles, Row::HowOften, Row::Enemies, Row::EnemyPower],
        ],
    );
    next_card(&mut list, view, card);
    on_window(&mut list, view, on);
    help_strip(&mut list, view.row);
    list
}

/// A panel of choices: its window, and each group with its title and its rows.
fn cheats_panel(list: &mut DrawList, view: &CheatsView, window: Rect, groups: &[&[Row]]) {
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + PANEL_SIDE;
    let right = window.right() - FRAME - PANEL_SIDE;
    let mut y = window.y + FRAME + PANEL_TOP;
    for rows in groups {
        y += GROUP_TITLE_TOP;
        let title = format!("-- {} --", rows[0].group());
        let middle = y + GROUP_TITLE_HEIGHT / 2.0;
        list.text(left + 26.0, middle, &title, GROUP_TEXT, palette::DARK_RED, Align::Left);
        y += GROUP_TITLE_HEIGHT + GROUP_TITLE_BELOW + LINE_GAP;
        for &row in *rows {
            cheat_row(list, view, row, left, right, y + ROW_HEIGHT / 2.0);
            y += ROW_HEIGHT + LINE_GAP;
        }
    }
}

/// One row of a panel: the cursor when it is under it, the label, and the row's choices at its right end (a
/// cycler for the "For" row).
fn cheat_row(list: &mut DrawList, view: &CheatsView, row: Row, left: f32, right: f32, middle: f32) {
    if view.row == row {
        list.push(Op::Cursor { x: left, y: middle - CURSOR_HEIGHT / 2.0 });
    }
    list.text(left + 26.0, middle, row.label(), ROW_TEXT, palette::BLACK, Align::Left);
    let texts: Vec<String> =
        (0..row.choices()).map(|choice| row.choice_text(choice, &view.names)).collect();
    let chosen = row.index(&view.cheats).min(texts.len().saturating_sub(1));
    if row == Row::ZFor {
        cycler(list, &texts[chosen], right, middle);
    } else {
        choices(list, &texts, chosen, right, middle);
    }
}

/// The choices of a row, from `right` leftwards, each in a button as wide as its text needs (at least
/// [`CHOICE_MIN`]). The chosen one is orange with white text and a dark-red line.
fn choices(list: &mut DrawList, texts: &[String], chosen: usize, right: f32, middle: f32) {
    let widths: Vec<f32> = texts
        .iter()
        .map(|text| (SMALL_TEXT * text.chars().count() as f32 + CHOICE_PADDING).max(CHOICE_MIN))
        .collect();
    let total = widths.iter().sum::<f32>() + LINE_GAP * widths.len().saturating_sub(1) as f32;
    let mut x = right - total;
    for (index, (text, width)) in texts.iter().zip(widths).enumerate() {
        let rect = Rect::new(x, middle - CHOICE_HEIGHT / 2.0, width, CHOICE_HEIGHT);
        let on = index == chosen;
        if on {
            list.fill(rect, palette::ORANGE);
            list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
        }
        let colour = if on { palette::WHITE } else { palette::BLACK };
        list.text(rect.x + width / 2.0, middle, text, SMALL_TEXT, colour, Align::Centre);
        x += width + LINE_GAP;
    }
}

/// A cycler: an arrow to the left, the value between two 4 px gaps, and an arrow to the right, the value
/// at least [`CYCLER_VALUE`] px wide and the whole ending at `right`.
fn cycler(list: &mut DrawList, value: &str, right: f32, middle: f32) {
    let width = (ROW_TEXT * value.chars().count() as f32).max(CYCLER_VALUE);
    let left = right - (2.0 * ARROW_BOX + 2.0 * CYCLER_GAP + width);
    let inset = (ARROW_BOX - CURSOR_WIDTH) / 2.0;
    let y = middle - CURSOR_HEIGHT / 2.0;
    list.push(Op::Arrow { x: left + inset, y, left: true });
    let value_left = left + ARROW_BOX + CYCLER_GAP;
    list.text(value_left + width / 2.0, middle, value, ROW_TEXT, palette::BLACK, Align::Centre);
    let arrow = value_left + width + CYCLER_GAP;
    list.push(Op::Arrow { x: arrow + inset, y, left: false });
}

/// The preview of the next card: its window, the card with its balls and its symbol, and under it the
/// corners the cards are Z in. The attack ball is the top left, the defence ball the bottom right.
fn next_card(list: &mut DrawList, view: &CheatsView, window: Rect) {
    list.push(Op::Window { rect: window });
    let title_middle = window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT / 2.0;
    let title_left = window.x + FRAME + WINDOW_SIDE;
    list.text(
        title_left,
        title_middle,
        "-- Next card --",
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    let card = Rect::new(
        window.x + (window.w - CARD_WIDTH) / 2.0,
        window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT + 10.0,
        CARD_WIDTH,
        CARD_HEIGHT,
    );
    // The card: a black line outside its white border, and a black face inside it.
    list.fill(card.grow(2.0), palette::BLACK);
    list.fill(card, palette::WHITE);
    let face = card.grow(-CARD_BORDER);
    list.fill(face, palette::BLACK);
    list.push(Op::Outline { rect: face.grow(-10.0), width: LINE, color: palette::CARD });
    let mode = view.cheats.z_mode();
    let attack = Rect::new(face.x + 2.0, face.y + 2.0, BALL, BALL);
    let defence = Rect::new(face.right() - 2.0 - BALL, face.bottom() - 2.0 - BALL, BALL, BALL);
    ball(list, attack, mode.attack(), true);
    ball(list, defence, mode.defence(), false);
    // The symbol in the middle: a kanji in a cream box with an orange border.
    let symbol = Rect::new(face.x + face.w / 2.0 - 29.0, face.y + 56.0, 58.0, 58.0);
    list.fill(symbol, palette::ORANGE);
    list.fill(symbol.grow(-4.0), palette::CREAM);
    list.text(symbol.x + 29.0, symbol.y + 29.0, "必", 40.0, palette::KI, Align::Centre);
    let caption = match (mode.attack(), mode.defence()) {
        (true, true) => "Z in attack and defence",
        (true, false) => "Z in attack",
        (false, true) => "Z in defence",
        (false, false) => "A card as the game deals it",
    };
    let first = card.bottom() + 6.0 + CAPTION_TEXT / 2.0;
    for (line, text) in wrap(caption, CAPTION_CHARS).iter().enumerate() {
        let y = first + 16.0 * line as f32;
        list.text(
            window.x + window.w / 2.0,
            y,
            text,
            CAPTION_TEXT,
            palette::DARK_RED,
            Align::Centre,
        );
    }
}

/// A ball of the card: a black ring around an orange disc, with a Z (when the corner is Z), four pips (an
/// attack that is not Z) or the kanji seven (a defence that is not Z) in it.
fn ball(list: &mut DrawList, at: Rect, zed: bool, attack: bool) {
    list.push(Op::Disc { rect: at, color: palette::BLACK });
    list.push(Op::Disc { rect: at.grow(-2.0), color: palette::CARD });
    let (x, y) = (at.x + at.w / 2.0, at.y + at.h / 2.0);
    if zed {
        // The Z of the design: M5 5 h14 L6 19 h14 in a 24 px box, drawn with a 4 px stroke.
        let (x0, y0) = (x - 12.0, y - 12.0);
        let point = |px: f32, py: f32| (x0 + px, y0 + py);
        let stroke = 4.0;
        let color = palette::BLACK;
        list.push(Op::Line { from: point(5.0, 5.0), to: point(19.0, 5.0), width: stroke, color });
        list.push(Op::Line { from: point(19.0, 5.0), to: point(6.0, 19.0), width: stroke, color });
        list.push(Op::Line { from: point(6.0, 19.0), to: point(20.0, 19.0), width: stroke, color });
    } else if attack {
        for (dx, dy) in [(-4.0, -4.0), (4.0, -4.0), (-4.0, 4.0), (4.0, 4.0)] {
            let pip = Rect::new(x + dx - 3.0, y + dy - 3.0, 6.0, 6.0);
            list.push(Op::Disc { rect: pip, color: palette::BLACK });
        }
    } else {
        list.text(x, y, "七", 22.0, palette::BLACK, Align::Centre);
    }
}

/// The On window: the number of settings on, each one's label and value, and Reset all at its foot (orange
/// while the cursor is on it).
fn on_window(list: &mut DrawList, view: &CheatsView, window: Rect) {
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + WINDOW_SIDE;
    let right = window.right() - FRAME - WINDOW_SIDE;
    let title = format!("-- On: {} --", view.cheats.on_count());
    let title_middle = window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT / 2.0;
    list.text(left, title_middle, &title, GROUP_TEXT, palette::DARK_RED, Align::Left);
    let button = Rect::new(
        left,
        window.bottom() - FRAME - WINDOW_TOP - RESET_HEIGHT,
        right - left,
        RESET_HEIGHT,
    );
    let first = window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT + 6.0;
    let lines = view.cheats.lines(&view.names);
    if lines.is_empty() {
        let text = "None. The game plays as the original.";
        for (line, part) in wrap(text, EMPTY_CHARS).iter().enumerate() {
            let middle = first + 11.0 + 22.0 * line as f32;
            list.text(left, middle, part, SMALL_TEXT, palette::BLACK, Align::Left);
        }
    } else {
        // The lines take the room above the button, 24 px apart at most and closer when there are many.
        let room = button.y - 6.0 - first;
        let pitch = (room / lines.len() as f32).min(24.0);
        for (line, (label, value)) in lines.iter().enumerate() {
            let middle = first + pitch * line as f32 + pitch / 2.0;
            // The value stops before the label, with a gap of 8 px, and is cut when the two would meet.
            let value = fit(value, SMALL_TEXT, right - left - text_width(label, SMALL_TEXT) - 8.0);
            list.text(left, middle, label, SMALL_TEXT, palette::BLACK, Align::Left);
            list.text(right, middle, &value, SMALL_TEXT, palette::BLACK, Align::Right);
        }
    }
    let focused = view.row == Row::ResetAll;
    list.fill(button, if focused { palette::ORANGE } else { palette::CREAM });
    list.push(Op::Outline { rect: button, width: LINE, color: palette::DARK_RED });
    let colour = if focused { palette::WHITE } else { palette::BLACK };
    list.text(
        button.x + button.w / 2.0,
        button.middle_y(),
        "Reset all",
        ROW_TEXT,
        colour,
        Align::Centre,
    );
}

/// The notice a cheat puts over the field: one window along the top of the picture, with its line of text.
#[must_use]
pub fn notice(text: &str) -> DrawList {
    let mut list = DrawList::new();
    let height = 2.0 * FRAME + 2.0 * PADDING + LARGE_TEXT;
    let window = Rect::new((WIDTH - NOTICE_WIDTH) / 2.0, MARGIN, NOTICE_WIDTH, height);
    list.push(Op::Window { rect: window });
    list.text(WIDTH / 2.0, window.middle_y(), text, LARGE_TEXT, palette::BLACK, Align::Centre);
    list
}

/// The width of a notice window.
const NOTICE_WIDTH: f32 = 640.0;

/// The original's save-slot lists, which mark a slot saved with a cheat on (task 3): the title's "Which slot?" list
/// (Continue) and the list of the Save, at its slot step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotList {
    Continue,
    Save,
}

/// The original's picture is its 256 x 239 pixels, centred in the 372-pixel-wide frame of the 16:9 view (with 58
/// pixels of the map at each side), so a pixel of the original lies at `(x + SNES_LEFT, y)` in that frame.
const SNES_LEFT: f32 = 58.0;
const FRAME_WIDTH: f32 = 372.0;
const FRAME_HEIGHT: f32 = 239.0;

/// Where the badge of `slot` goes on `list`, in the original's pixels (left, top, right, bottom): on the Continue list
/// right of the slot's tab (the tabs' tops are 48, 104 and 160); on the Save list right of the slot's row (the rows
/// are centred at 186, 202 and 218, beside the window's right edge).
fn badge_rect(list: SlotList, slot: usize) -> (f32, f32, f32, f32) {
    match list {
        SlotList::Continue => {
            let top = [48.0, 104.0, 160.0][slot] + 15.0;
            (207.0, top, 236.0, top + 14.0)
        }
        SlotList::Save => {
            let middle = [186.0, 202.0, 218.0][slot];
            (206.0, middle - 7.0, 236.0, middle + 7.0)
        }
    }
}

/// The CHEATS badges of a slot list: a tag beside each slot whose save was made with a cheat on (`marked`).
#[must_use]
pub fn slot_badges(list: SlotList, marked: [bool; 3]) -> DrawList {
    let mut out = DrawList::new();
    for (slot, &on) in marked.iter().enumerate() {
        if !on {
            continue;
        }
        let (left, top, right, bottom) = badge_rect(list, slot);
        let scale_x = WIDTH / FRAME_WIDTH;
        let scale_y = HEIGHT / FRAME_HEIGHT;
        let rect = Rect::new(
            (left + SNES_LEFT) * scale_x,
            top * scale_y,
            (right - left) * scale_x,
            (bottom - top) * scale_y,
        );
        out.fill(rect, palette::CREAM);
        out.push(Op::Outline { rect, width: LINE, color: palette::ORANGE });
        out.text(
            rect.x + rect.w / 2.0,
            rect.middle_y(),
            "CHEATS",
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Centre,
        );
    }
    out
}

/// The help of the row under the cursor, along the foot, with the key that goes back.
fn help_strip(list: &mut DrawList, row: Row) {
    let window =
        Rect::new(MARGIN, HEIGHT - CHEATS_EDGE - CHEATS_HELP, WIDTH - 2.0 * MARGIN, CHEATS_HELP);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 20.0;
    list.text(left, window.middle_y(), row.help(), ROW_TEXT, palette::BLACK, Align::Left);
    let right = window.right() - FRAME - 20.0;
    list.text(right, window.middle_y(), "B  Back", SMALL_TEXT, palette::DARK_RED, Align::Right);
}

/// The words of `text` in lines of at most `width` characters; a word longer than a line has a line of its own.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cheats::{Cheats, Setting, ZMode, ZTarget};
    use crate::view::{Member, Slot};

    fn field_view(cursor: u8) -> MenuView {
        MenuView {
            screen: Screen::Command,
            bag: None,
            town: false,
            flying: false,
            cursor,
            page: 0,
            zeni: 40,
            party: vec![
                Member {
                    name: "Goku".into(),
                    level: 10,
                    hp: 80,
                    max_hp: 100,
                    ki: 12,
                    max_ki: 12,
                    bp: 416,
                    face: Some(28),
                },
                Member {
                    name: "Piccolo".into(),
                    level: 2,
                    hp: 77,
                    max_hp: 77,
                    ki: 9,
                    max_ki: 12,
                    bp: 416,
                    face: None,
                },
            ],
            items: Vec::new(),
            selected: None,
        }
    }

    fn items_view(count: usize, page: u8, cursor: u8) -> MenuView {
        let items = (0..count)
            .map(|i| Slot { object: i as u8, attribute: 0xC0, name: format!("Item{i}") })
            .collect();
        MenuView {
            screen: Screen::Items,
            items,
            page,
            cursor,
            selected: Some(Selected {
                object: 2,
                name: "Bulma".into(),
                price: Some(3),
                help: vec!["Restores 25".into(), "HP.".into()],
                battle: true,
                field: true,
            }),
            ..field_view(0)
        }
    }

    fn texts(list: &DrawList) -> Vec<&str> {
        list.texts().map(|(_, _, text)| text).collect()
    }

    fn cursors(list: &DrawList) -> Vec<(f32, f32)> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::Cursor { x, y } => Some((*x, *y)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_command_window_lists_the_field_commands_with_the_cursor_on_one() {
        let list = command(&field_view(2));
        for label in ["Talk", "Look", "Fly", "Item", "Menu"] {
            assert!(texts(&list).contains(&label), "{label}");
        }
        let cursor = cursors(&list);
        assert_eq!(cursor.len(), 1, "one cursor");
        // The rows are 44 px apart and the cursor is on the third one.
        let first = cursors(&command(&field_view(0)))[0].1;
        assert!((cursor[0].1 - first - 2.0 * COMMAND_ROW).abs() < 1e-3);
    }

    #[test]
    fn flying_and_town_have_their_own_commands() {
        let mut view = field_view(0);
        view.flying = true;
        let names: Vec<&str> = commands(&view).iter().map(|c| c.label()).collect();
        assert_eq!(names, ["Land", "Item", "Menu"]);
        view.flying = false;
        view.town = true;
        let names: Vec<&str> = commands(&view).iter().map(|c| c.label()).collect();
        assert_eq!(names, ["Talk", "Look", "Item", "Menu"]);
    }

    #[test]
    fn the_menu_list_shows_its_entries_and_greys_only_the_one_that_waits_for_later() {
        let mut view = field_view(6);
        view.screen = Screen::Menu;
        let list = scene(&view);
        let text = texts(&list);
        for label in ["Cards", "Status", "Order", "Text", "Save", "Cheats", "Options"] {
            assert!(text.contains(&label), "{label} in {text:?}");
        }
        assert_eq!(text.iter().filter(|t| **t == "coming soon").count(), 1);
        let grey = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Text { color, .. } if *color == palette::GREY))
            .count();
        assert_eq!(grey, 2, "the label and its note in grey");
        assert!(text.contains(&"Coming soon."), "the help of the greyed entry under the cursor");
        assert_eq!(cursors(&list).len(), 1, "the cursor is on Options");
        view.cursor = 5;
        assert!(texts(&scene(&view)).contains(&"Set the cheats of this run."));
    }

    #[test]
    fn the_backdrop_is_the_first_step_and_the_money_and_the_party_are_drawn() {
        let list = command(&field_view(0));
        assert_eq!(list.ops().first(), Some(&Op::Backdrop));
        let text = texts(&list);
        assert!(text.contains(&"Zeni 040P"), "{text:?}");
        let portraits: Vec<Option<u8>> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Portrait { object, .. } => Some(*object),
                _ => None,
            })
            .collect();
        assert_eq!(
            portraits,
            [Some(28), None],
            "one frame per corner member, empty without a face"
        );
    }

    #[test]
    fn the_item_page_shows_its_eight_entries_and_the_cursor_on_its_row() {
        let list = items(&items_view(12, 1, 2));
        let text = texts(&list);
        assert!(text.contains(&"Item8") && text.contains(&"Item11"), "{text:?}");
        assert!(!text.contains(&"Item7") && !text.contains(&"Item12"), "{text:?}");
        assert!(text.contains(&"Page 2/2"), "{text:?}");
        assert_eq!(cursors(&list).len(), 1);
    }

    #[test]
    fn the_header_counts_against_the_capacity_of_24() {
        let list = items(&items_view(5, 0, 0));
        assert!(texts(&list).contains(&"05/24"));
    }

    #[test]
    fn the_price_box_shows_the_price_and_its_unit_or_nothing_and_the_help_follows_the_name() {
        let mut view = items_view(1, 0, 0);
        let list = items(&view);
        let text = texts(&list);
        assert!(text.contains(&"3") && text.contains(&"Bulma"), "{text:?}");
        assert!(text.contains(&"Restores 25") && text.contains(&"HP."), "{text:?}");
        assert!(text.contains(&"Pt"), "{text:?}");
        // An object with no price: no figure, no dash, no unit
        view.selected.as_mut().unwrap().price = None;
        let list = items(&view);
        let text = texts(&list);
        assert!(!text.contains(&"-") && !text.contains(&"Pt") && !text.contains(&"3"), "{text:?}");
        assert!(text.contains(&"Bulma") && text.contains(&"Restores 25"), "{text:?}");
    }

    #[test]
    fn every_window_lies_inside_the_space() {
        let mut menu = field_view(6);
        menu.screen = Screen::Menu;
        for list in [command(&field_view(0)), menu_list(&menu), items(&items_view(24, 2, 7))] {
            for op in list.ops() {
                if let Op::Window { rect } = op {
                    assert!(rect.x >= 0.0 && rect.y >= 0.0, "{rect:?}");
                    assert!(rect.right() <= WIDTH && rect.bottom() <= HEIGHT, "{rect:?}");
                }
            }
        }
    }

    #[test]
    fn the_quantity_sheet_draws_its_arrows_as_shapes_around_the_figure() {
        let bag = BagView {
            tab: Tab::All,
            stacks: Vec::new(),
            used: 0,
            slots: 60,
            full: false,
            usable_here: true,
            droppable_here: true,
            sheet: Sheet::Quantity,
            choice: 0,
            member: 0,
            units: 7,
            most: 9,
            notice: None,
        };
        let window = sheet_window(&bag, &[]).expect("the quantity sheet is a sheet");
        let mut list = DrawList::new();
        sheet_draw(&mut list, &window);
        let arrows: Vec<(f32, bool)> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Arrow { x, left, .. } => Some((*x, *left)),
                _ => None,
            })
            .collect();
        assert_eq!(arrows.len(), 2, "a left and a right arrow");
        assert!(arrows[0].1 && !arrows[1].1);
        let figure = list.texts().find(|(_, _, text)| text.contains('7')).expect("the figure");
        assert_eq!(figure.2, "\u{d7}07");
        assert!(arrows[0].0 < figure.0 && figure.0 < arrows[1].0, "the figure is between them");
        assert!(list.texts().all(|(_, _, text)| !text.contains(['<', '>'])), "no ASCII stand-ins");
    }

    #[test]
    fn the_use_on_list_shows_level_hp_ki_and_bp() {
        let view = field_view(0);
        let bag = BagView {
            tab: Tab::All,
            stacks: Vec::new(),
            used: 0,
            slots: 60,
            full: false,
            usable_here: true,
            droppable_here: true,
            sheet: Sheet::Members,
            choice: 0,
            member: 0,
            units: 1,
            most: 1,
            notice: None,
        };
        let window = sheet_window(&bag, &view.party).expect("the members sheet");
        let figures = window.rows[0].1.as_deref().expect("the figures of the first member");
        for part in ["LV 10", "HP  80/100", "KI  12/12", "BP 416"] {
            assert!(figures.contains(part), "{part} in {figures}");
        }
    }

    #[test]
    fn the_use_on_sheet_of_a_full_party_ends_above_the_margin() {
        let members = |count: usize| -> Vec<Member> {
            (0..count)
                .map(|i| Member {
                    name: format!("Member {i}"),
                    level: 1,
                    hp: 1,
                    max_hp: 1,
                    ki: 1,
                    max_ki: 1,
                    bp: 1,
                    face: None,
                })
                .collect()
        };
        let bag = || BagView {
            tab: Tab::All,
            stacks: Vec::new(),
            used: 0,
            slots: 60,
            full: false,
            usable_here: true,
            droppable_here: true,
            sheet: Sheet::Members,
            choice: 0,
            member: 0,
            units: 1,
            most: 1,
            notice: None,
        };
        // Two or seven members keep the rows 40 px apart, as the design has them.
        for count in [2, 7] {
            let party = members(count);
            let window = sheet_window(&bag(), &party).expect("the members sheet");
            assert!((window.pitch - SHEET_ROW).abs() < 1e-3, "{count} members: {:?}", window.pitch);
        }
        // Nine (the most the records hold) are closer, and the last row is still inside the sheet.
        let party = members(9);
        let window = sheet_window(&bag(), &party).expect("the members sheet");
        assert!(window.pitch < SHEET_ROW, "{:?}", window.pitch);
        assert!(window.rect.bottom() <= HEIGHT - MARGIN, "{:?}", window.rect);
        let mut list = DrawList::new();
        sheet_draw(&mut list, &window);
        let last = list.texts().find(|(_, _, text)| *text == "Member 8").expect("the last row");
        assert!(last.1 + LARGE_TEXT / 2.0 <= window.rect.bottom(), "{last:?} in {:?}", window.rect);
    }

    #[test]
    fn the_gauge_fraction_is_clamped_and_zero_without_a_maximum() {
        let mut list = DrawList::new();
        let row = GaugeRow { x: 0.0, y: 0.0, width: 104.0, label: "HP", fill: palette::HP };
        gauge_row(&mut list, row, (150, 100));
        gauge_row(
            &mut list,
            GaugeRow { x: 0.0, y: 0.0, width: 104.0, label: "KI", fill: palette::KI },
            (0, 0),
        );
        let fractions: Vec<f32> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Bar { fraction, .. } => Some(*fraction),
                _ => None,
            })
            .collect();
        assert_eq!(fractions, [1.0, 0.0]);
        assert!((BAR_HEIGHT - 12.0).abs() < 1e-3);
    }

    fn cheats_view(row: Row, cheats: Cheats) -> CheatsView {
        CheatsView { cheats, row, names: (0..12).map(|id| format!("Fighter {id}")).collect() }
    }

    fn has_text(list: &DrawList, text: &str, colour: Color) -> bool {
        list.ops().iter().any(
            |op| matches!(op, Op::Text { text: t, color, .. } if t == text && *color == colour),
        )
    }

    #[test]
    fn every_window_and_text_of_the_cheats_screen_lies_inside_the_space() {
        let mut busy = Cheats::neutral();
        for setting in Setting::ALL {
            busy.set_index(setting, setting.steps().len() - 1);
        }
        busy.set_random_battles_off(true);
        busy.set_z_mode(ZMode::Both);
        busy.set_z_target(ZTarget::Fighter(11));
        busy.set_zeni(true);
        busy.set_endless(true);
        for row in Row::ALL {
            for cheats in [Cheats::neutral(), busy] {
                let list = cheats_screen(&cheats_view(row, cheats));
                for op in list.ops() {
                    if let Op::Window { rect } = op {
                        assert!(rect.x >= MARGIN - 1e-3 && rect.right() <= WIDTH - MARGIN + 1e-3);
                        assert!(rect.y >= 0.0 && rect.bottom() <= HEIGHT, "{rect:?}");
                    }
                }
                for (x, y, text) in list.texts() {
                    assert!((0.0..=WIDTH).contains(&x) && (0.0..=HEIGHT).contains(&y), "{text}");
                }
            }
        }
    }

    #[test]
    fn the_chosen_choice_is_orange_with_white_text_and_the_others_are_black() {
        let mut cheats = Cheats::neutral();
        cheats.set_index(Setting::Damage, 2);
        let list = cheats_screen(&cheats_view(Row::Damage, cheats));
        assert!(has_text(&list, "×2", palette::WHITE), "the chosen Damage is white");
        assert!(has_text(&list, "×1", palette::BLACK), "Defence's neutral choice is black");
        let highlights = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Fill { rect, color } if *color == palette::ORANGE && rect.x < 960.0))
            .count();
        assert_eq!(highlights, 10, "one highlight in each row of choices, the For cycler has none");
    }

    #[test]
    fn the_for_row_cycles_through_the_party_with_the_name_of_its_choice() {
        let mut cheats = Cheats::neutral();
        cheats.set_z_target(ZTarget::Fighter(1));
        let list = cheats_screen(&cheats_view(Row::ZFor, cheats));
        assert!(texts(&list).contains(&"Fighter 1"), "{:?}", texts(&list));
        let arrows: Vec<bool> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Arrow { left, .. } => Some(*left),
                _ => None,
            })
            .collect();
        assert_eq!(arrows, [true, false], "a left and a right arrow");
        assert_eq!(cursors(&list).len(), 1, "the cursor is on the For row");
        let whole = cheats_screen(&cheats_view(Row::ZFor, Cheats::neutral()));
        assert!(texts(&whole).contains(&"Whole party"));
    }

    #[test]
    fn the_card_shows_a_z_in_each_corner_that_is_z_and_its_caption_says_so() {
        let card = |mode: ZMode| {
            let mut cheats = Cheats::neutral();
            cheats.set_z_mode(mode);
            cheats_screen(&cheats_view(Row::Damage, cheats))
        };
        let strokes =
            |list: &DrawList| list.ops().iter().filter(|op| matches!(op, Op::Line { .. })).count();
        assert_eq!(strokes(&card(ZMode::Off)), 0);
        assert_eq!(strokes(&card(ZMode::Attack)), 3, "a Z is three strokes");
        assert_eq!(strokes(&card(ZMode::Defence)), 3);
        assert_eq!(strokes(&card(ZMode::Both)), 6);
        let off = card(ZMode::Off);
        assert!(texts(&off).contains(&"七"), "the defence of a card that is not Z");
        assert!(texts(&off).contains(&"A card as the"), "{:?}", texts(&off));
        let attack = card(ZMode::Attack);
        assert!(texts(&attack).contains(&"Z in attack"), "{:?}", texts(&attack));
        assert!(texts(&attack).contains(&"七"), "the defence stays a number");
        let defence = card(ZMode::Defence);
        assert!(!texts(&defence).contains(&"七"), "a Z defence has no number");
        assert_eq!(wrap("Z in attack and defence", CAPTION_CHARS), ["Z in attack and", "defence"]);
    }

    #[test]
    fn the_on_window_counts_what_is_on_and_says_none_when_nothing_is() {
        let off = cheats_screen(&cheats_view(Row::ResetAll, Cheats::neutral()));
        assert!(texts(&off).contains(&"-- On: 0 --"));
        assert!(texts(&off).contains(&"None. The game"), "{:?}", texts(&off));
        assert!(has_text(&off, "Reset all", palette::WHITE), "the cursor is on Reset all");
        let mut on = Cheats::neutral();
        on.set_index(Setting::Damage, 2);
        on.set_zeni(true);
        let list = cheats_screen(&cheats_view(Row::Damage, on));
        assert!(texts(&list).contains(&"-- On: 2 --"));
        assert!(texts(&list).contains(&"Zeni") && texts(&list).contains(&"max"));
        assert!(has_text(&list, "Reset all", palette::BLACK), "not focused: black on cream");
    }

    #[test]
    fn the_help_line_is_the_help_of_the_row_under_the_cursor() {
        for row in Row::ALL {
            let list = cheats_screen(&cheats_view(row, Cheats::neutral()));
            assert!(texts(&list).contains(&row.help()), "{row:?}");
        }
    }

    #[test]
    fn a_notice_is_one_window_along_the_top_with_its_line_centred() {
        let list = notice("Cheats off for this fight");
        let windows: Vec<Rect> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Window { rect } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(windows.len(), 1, "one window");
        let window = windows[0];
        assert!((window.x + window.w / 2.0 - WIDTH / 2.0).abs() < 1e-3, "centred: {window:?}");
        assert!(window.y >= MARGIN - 1e-3 && window.bottom() < HEIGHT / 4.0, "{window:?}");
        assert!(texts(&list).contains(&"Cheats off for this fight"));
    }

    #[test]
    fn the_cheats_badges_mark_each_slot_saved_with_a_cheat_inside_the_space() {
        for list in [SlotList::Continue, SlotList::Save] {
            assert!(slot_badges(list, [false; 3]).ops().is_empty(), "no badge without a cheat");
            let all = slot_badges(list, [true; 3]);
            let badges = all.texts().filter(|(_, _, text)| *text == "CHEATS").count();
            assert_eq!(badges, 3, "{list:?}: one badge a slot");
            for op in all.ops() {
                if let Op::Fill { rect, .. } = op {
                    assert!(rect.x >= 0.0 && rect.right() <= WIDTH, "{rect:?}");
                    assert!(rect.y >= 0.0 && rect.bottom() <= HEIGHT, "{rect:?}");
                }
            }
            let one = slot_badges(list, [false, true, false]);
            assert_eq!(one.texts().count(), 1, "only the marked slot");
        }
        // The Continue list's first tag is below the first slot's tab, the Save list's second one at its second row.
        let continue_first = slot_badges(SlotList::Continue, [true, false, false]);
        let (_, first_y, _) = continue_first.texts().next().expect("the badge text");
        assert!(
            first_y > 63.0 * HEIGHT / FRAME_HEIGHT - 1.0 && first_y < HEIGHT / 2.0,
            "{first_y}"
        );
        let save_second = slot_badges(SlotList::Save, [false, true, false]);
        let (_, second_y, _) = save_second.texts().next().expect("the badge text");
        assert!((second_y - 202.0 * HEIGHT / FRAME_HEIGHT).abs() < 1.0, "{second_y}");
    }

    #[test]
    fn the_words_of_a_caption_wrap_at_the_width_given() {
        assert_eq!(wrap("A card as the game deals it", 15), ["A card as the", "game deals it"]);
        assert_eq!(wrap("", 15), Vec::<String>::new());
        assert_eq!(wrap("averyveryverylongword", 5), ["averyveryverylongword"]);
    }
}
