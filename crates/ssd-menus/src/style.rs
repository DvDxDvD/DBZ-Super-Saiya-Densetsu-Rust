//! The accessibility style of the HD menus (per PC): the text size, the window colours and the cursor. The screens
//! draw the design as it is; [`Style::apply`] turns a finished draw list into the list a player with these choices
//! sees, and returns it untouched for the default style.

use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{CURSOR_HEIGHT, CURSOR_WIDTH, FRAME, LINE, palette, text_width};

/// How much the large text size grows the text.
pub const LARGE_FACTOR: f32 = 1.25;
/// The blue of the safe cursor.
pub const SAFE_BLUE: Color = Color::rgb(0x00, 0x57, 0xB8);
/// The width and height of the safe cursor's arrow.
pub const SAFE_WIDTH: f32 = 24.0;
pub const SAFE_HEIGHT: f32 = 26.0;
/// Where the safe cursor's bar lies under the middle of its row, and how thick it is.
const BAR_BELOW: f32 = 16.0;
const BAR_THICK: f32 = 3.0;
/// How far the safe arrow lies left of the cursor's centre: it is wider, and the text starts where the cursor ends.
const SAFE_SHIFT: f32 = 3.0;
/// The text of an entry that is greyed out, in high contrast: dark enough to read on white.
const HC_GREY: Color = Color::rgb(0x55, 0x55, 0x55);
/// The tint of a row under the cursor, in high contrast.
const HC_TINT: Color = Color([0, 0, 0, 40]);
/// Half the height of a line of text, in ems: a text box is a little taller than its size.
const HALF_LINE: f32 = 0.57;
/// The room kept between a grown text and what lies beside it.
const SPARE: f32 = 4.0;

/// How the HD menus look for the three accessibility choices. The default is the design as drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    /// Large text: every text a quarter bigger, where there is room beside it and inside its window.
    pub large_text: bool,
    /// High contrast: white windows, black text and frame, no orange.
    pub contrast: bool,
    /// The safe cursor: a bigger blue arrow with a bar under the row.
    pub safe_cursor: bool,
}

/// A text run's horizontal extent.
fn extent(x: f32, text: &str, size: f32, align: Align) -> (f32, f32) {
    let width = text_width(text, size);
    match align {
        Align::Left => (x, x + width),
        Align::Centre => (x - width / 2.0, x + width / 2.0),
        Align::Right => (x - width, x),
    }
}

/// Whether (`x`, `y`) is inside `rect`.
fn inside(rect: &Rect, x: f32, y: f32) -> bool {
    rect.x <= x && x <= rect.right() && rect.y <= y && y <= rect.bottom()
}

/// The smallest of `windows` that holds (`x`, `y`).
fn innermost(windows: &[Rect], x: f32, y: f32) -> Option<Rect> {
    windows
        .iter()
        .filter(|w| inside(w, x, y))
        .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
        .copied()
}

impl Style {
    /// Whether this is the design as drawn.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// `list` as this style shows it. The default returns it as it is.
    #[must_use]
    pub fn apply(&self, list: DrawList) -> DrawList {
        if self.is_default() {
            return list;
        }
        let ops = list.ops();
        let windows: Vec<Rect> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Window { rect } => Some(*rect),
                _ => None,
            })
            .collect();
        let sizes = if self.large_text { text_sizes(ops, &windows) } else { Vec::new() };
        let mut out = DrawList::new();
        // The fills turned black by high contrast: a text on one of them is white.
        let mut dark: Vec<Rect> = Vec::new();
        for (index, op) in ops.iter().enumerate() {
            match op {
                Op::Window { rect } if self.contrast => out.push(Op::WindowHc { rect: *rect }),
                Op::Fill { rect, color } if self.contrast => {
                    let color = tint(*color);
                    if color == palette::BLACK {
                        dark.push(*rect);
                    }
                    out.fill(*rect, color);
                }
                Op::Outline { rect, width, color } if self.contrast => {
                    out.push(Op::Outline { rect: *rect, width: *width, color: tint(*color) });
                }
                Op::Line { from, to, width, color } if self.contrast => {
                    out.push(Op::Line { from: *from, to: *to, width: *width, color: tint(*color) });
                }
                Op::Text { x, y, text, size, color, align } => {
                    let size = sizes.get(index).copied().unwrap_or(*size);
                    let mut color = *color;
                    if self.contrast {
                        color = tint(color);
                        if color == palette::BLACK && dark.iter().any(|r| inside(r, *x, *y)) {
                            color = palette::WHITE;
                        }
                    }
                    out.push(Op::Text {
                        x: *x,
                        y: *y,
                        text: text.clone(),
                        size,
                        color,
                        align: *align,
                    });
                }
                Op::Cursor { x, y } if self.safe_cursor => {
                    let tint = index.checked_sub(1).and_then(|before| match &ops[before] {
                        Op::Fill { rect, color } if color.0[3] < 255 => Some(*rect),
                        _ => None,
                    });
                    safe_cursor(&mut out, &windows, tint.filter(|r| inside(r, *x, *y)), *x, *y);
                }
                Op::Cursor { x, y } if self.contrast => out.push(black_arrow(*x, *y, false)),
                Op::Arrow { x, y, left } if self.contrast => out.push(black_arrow(*x, *y, *left)),
                other => out.push(other.clone()),
            }
        }
        out
    }
}

/// The colour `color` turns into in high contrast.
fn tint(color: Color) -> Color {
    let Color([r, g, b, a]) = color;
    let same = |c: Color| c.0[..3] == [r, g, b];
    if same(palette::CREAM) {
        Color([0xFF, 0xFF, 0xFF, a])
    } else if same(palette::ORANGE) {
        if a < 255 { HC_TINT } else { palette::BLACK }
    } else if same(palette::DARK_RED) || same(palette::CAP) {
        Color([0, 0, 0, a])
    } else if same(palette::GREY) {
        HC_GREY
    } else {
        color
    }
}

/// The arrow of high contrast: black with a white line, the cursor's size.
fn black_arrow(x: f32, y: f32, left: bool) -> Op {
    Op::Pointer {
        rect: Rect::new(x, y, CURSOR_WIDTH, CURSOR_HEIGHT),
        left,
        fill: palette::BLACK,
        edge: palette::WHITE,
        ring: palette::BLACK,
    }
}

/// The safe cursor at the cursor whose top-left corner is (`x`, `y`): a bigger blue arrow, and a blue bar
/// along the foot of the row. `tint` is the tint of the row the cursor is on, when the screen drew one (the bar
/// takes its width and its foot); else the bar crosses the window the cursor is in, a little under the arrow.
fn safe_cursor(out: &mut DrawList, windows: &[Rect], tint: Option<Rect>, x: f32, y: f32) {
    let middle = y + CURSOR_HEIGHT / 2.0;
    let centre = x + CURSOR_WIDTH / 2.0 - SAFE_SHIFT;
    out.push(Op::Pointer {
        rect: Rect::new(
            centre - SAFE_WIDTH / 2.0,
            middle - SAFE_HEIGHT / 2.0,
            SAFE_WIDTH,
            SAFE_HEIGHT,
        ),
        left: false,
        fill: SAFE_BLUE,
        edge: palette::BLACK,
        ring: palette::WHITE,
    });
    let bar = match (tint, innermost(windows, x, middle)) {
        (Some(row), _) => Some(Rect::new(row.x, row.bottom() - BAR_THICK, row.w, BAR_THICK)),
        (None, Some(window)) => {
            let inset = FRAME + LINE + SPARE;
            Some(Rect::new(window.x + inset, middle + BAR_BELOW, window.w - 2.0 * inset, BAR_THICK))
        }
        (None, None) => None,
    };
    if let Some(bar) = bar {
        out.fill(bar, SAFE_BLUE);
    }
}

/// One text run seen by [`text_sizes`].
struct Run {
    index: usize,
    x: f32,
    y: f32,
    size: f32,
    align: Align,
    left: f32,
    right: f32,
}

/// The size of each text run of `ops` (by its index in the list; other steps hold 0) when the text grows: a
/// quarter bigger, less where the neighbour on the row or the window's edge leaves no room, and never smaller
/// than drawn.
fn text_sizes(ops: &[Op], windows: &[Rect]) -> Vec<f32> {
    let mut sizes = vec![0.0; ops.len()];
    let runs: Vec<Run> = ops
        .iter()
        .enumerate()
        .filter_map(|(index, op)| match op {
            Op::Text { x, y, text, size, align, .. } => {
                let (left, right) = extent(*x, text, *size, *align);
                Some(Run { index, x: *x, y: *y, size: *size, align: *align, left, right })
            }
            _ => None,
        })
        .collect();
    for run in &runs {
        let (mut least, mut most) = (f32::MIN, f32::MAX);
        let mut tall = f32::MAX;
        if let Some(w) = innermost(windows, run.x, run.y) {
            let paper = FRAME + LINE;
            tall = (run.y - w.y - paper).min(w.bottom() - paper - run.y) / HALF_LINE;
            least = w.x + FRAME + LINE + SPARE;
            most = w.right() - FRAME - LINE - SPARE;
        }
        for other in runs.iter().filter(|o| o.index != run.index) {
            if (other.y - run.y).abs() >= 0.5 * run.size.min(other.size) {
                continue;
            }
            if other.left >= run.right - 0.5 && other.left > run.left {
                most = most.min(run.right + (other.left - run.right) / 2.0);
            } else if other.right <= run.left + 0.5 && other.right < run.right {
                least = least.max(run.left - (run.left - other.right) / 2.0);
            }
        }
        let width = run.right - run.left;
        let room = match run.align {
            Align::Left => most - run.left,
            Align::Right => run.right - least,
            Align::Centre => 2.0 * (run.x - least).min(most - run.x),
        };
        let grown = if width <= 0.0 { run.size * LARGE_FACTOR } else { run.size * room / width };
        let grown = grown.min(tall);
        sizes[run.index] = grown.clamp(run.size, run.size * LARGE_FACTOR);
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DrawList {
        let mut list = DrawList::new();
        list.push(Op::Window { rect: Rect::new(40.0, 40.0, 400.0, 200.0) });
        list.push(Op::Cursor { x: 70.0, y: 90.0 });
        list.text(100.0, 100.0, "Items", 30.0, palette::BLACK, Align::Left);
        list.text(420.0, 100.0, "12", 16.0, palette::DARK_RED, Align::Right);
        list
    }

    fn sizes_of(list: &DrawList) -> Vec<f32> {
        list.ops()
            .iter()
            .filter_map(|op| if let Op::Text { size, .. } = op { Some(*size) } else { None })
            .collect()
    }

    #[test]
    fn the_default_style_returns_the_list_as_it_is() {
        let list = sample();
        assert_eq!(Style::default().apply(list.clone()), list);
    }

    #[test]
    fn large_text_grows_the_runs_a_quarter_and_never_past_the_window() {
        let style = Style { large_text: true, ..Style::default() };
        let sizes = sizes_of(&style.apply(sample()));
        assert!((sizes[0] - 37.5).abs() < 1e-3 && (sizes[1] - 20.0).abs() < 1e-3, "{sizes:?}");
        // A run with no room beside it keeps its size.
        let mut tight = DrawList::new();
        tight.push(Op::Window { rect: Rect::new(0.0, 0.0, 120.0, 100.0) });
        tight.text(10.0, 50.0, "Dragon Radar", 16.0, palette::BLACK, Align::Left);
        assert!((sizes_of(&style.apply(tight))[0] - 16.0).abs() < 1e-3);
    }

    #[test]
    fn a_neighbour_on_the_row_limits_the_growth() {
        let mut list = DrawList::new();
        list.text(10.0, 50.0, "Name of it", 20.0, palette::BLACK, Align::Left);
        list.text(130.0, 50.0, "x99", 20.0, palette::BLACK, Align::Left);
        let sizes = sizes_of(&Style { large_text: true, ..Style::default() }.apply(list));
        // Ten characters at 20 px are 108 wide, ending at 118: half the 12 px gap is all the room (about 5% more).
        assert!(sizes[0] < 22.0, "{sizes:?}");
    }

    #[test]
    fn high_contrast_turns_windows_white_and_the_orange_black_with_white_text_on_it() {
        let mut list = DrawList::new();
        list.push(Op::Window { rect: Rect::new(0.0, 0.0, 300.0, 100.0) });
        list.fill(Rect::new(10.0, 10.0, 100.0, 40.0), palette::ORANGE);
        list.text(60.0, 30.0, "On", 16.0, palette::BLACK, Align::Centre);
        list.text(60.0, 80.0, "Off", 16.0, palette::BLACK, Align::Centre);
        let out = Style { contrast: true, ..Style::default() }.apply(list);
        assert!(matches!(out.ops()[0], Op::WindowHc { .. }));
        assert_eq!(
            out.ops()[1],
            Op::Fill { rect: Rect::new(10.0, 10.0, 100.0, 40.0), color: palette::BLACK }
        );
        let colours: Vec<Color> = out
            .ops()
            .iter()
            .filter_map(|op| if let Op::Text { color, .. } = op { Some(*color) } else { None })
            .collect();
        assert_eq!(colours, [palette::WHITE, palette::BLACK]);
    }

    #[test]
    fn the_safe_cursor_is_a_blue_arrow_with_a_bar_under_the_row() {
        let out = Style { safe_cursor: true, ..Style::default() }.apply(sample());
        let has = |f: fn(&Op) -> bool| out.ops().iter().any(f);
        assert!(has(|op| matches!(op, Op::Pointer { fill, .. } if *fill == SAFE_BLUE)));
        assert!(has(|op| matches!(op, Op::Fill { color, .. } if *color == SAFE_BLUE)));
        assert!(!has(|op| matches!(op, Op::Cursor { .. })));
    }
}
