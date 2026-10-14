//! Putting the layers together: priorities, windows, the main and sub screens, color math and
//! brightness, for the 256-pixel modes.

use crate::bg::bg_bitplanes;
use crate::{Binder, Pixel, Renderer, Source, WIDTH};
use ssd_oracle::{Color, LineRegs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layer {
    Bg1,
    Bg2,
    Bg3,
    Bg4,
    Obj,
    Backdrop,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Rendered {
    pub(crate) color: u16,
    pub(crate) palette: u8,
    pub(crate) layer: Layer,
    pub(crate) char: u16,
    /// [`Pixel::pos`] and [`Pixel::oam`] of the pixel shown.
    pub(crate) pos: u8,
    pub(crate) oam: u8,
    /// The priority number the pixel won with (lower is in front); `u8::MAX` for the backdrop.
    pub(crate) priority: u8,
}

impl Rendered {
    const BACKDROP: Self = Self {
        color: 0,
        palette: 0,
        layer: Layer::Backdrop,
        char: 0,
        pos: 0,
        oam: crate::NO_OBJECT,
        priority: u8::MAX,
    };
}

/// The frontmost pixel of a screen so far: the lowest priority number wins.
#[derive(Debug, Clone, Copy)]
struct Resolver {
    priority: u8,
    pixel: Pixel,
    layer: Layer,
}

impl Resolver {
    const NONE: Self =
        Self { priority: u8::MAX, pixel: Pixel::TRANSPARENT, layer: Layer::Backdrop };

    // Mode 0 and 1: OBJ.3 > BG1.1 > BG2.1 > OBJ.2 > BG1.0 > BG2.0 > OBJ.1 > BG3.1 > BG4.1 >
    // OBJ.0 > BG3.0 > BG4.0 (BG3.1 first when mode 1 sets the BG3 priority bit).
    // Mode 7 (EXTBG): OBJ.3 > BG1.1 > OBJ.2 > BG2.1 > OBJ.1 > BG1.0 > OBJ.0 > BG2.0.
    fn add(&mut self, pixel: Pixel, layer: Layer, priority: u8) {
        if priority < self.priority {
            *self = Self { priority, pixel, layer };
        }
    }

    fn add_obj(&mut self, pixel: Pixel, mode_01: bool) {
        let priority = match (mode_01, pixel.priority) {
            (_, 3) => 0,
            (true, 2) => 3,
            (true, 1) => 6,
            (true, _) => 9,
            (false, 2) => 2,
            (false, 1) => 4,
            (false, _) => 6,
        };
        self.add(pixel, Layer::Obj, priority);
    }

    fn add_bg1(&mut self, pixel: Pixel, mode_01: bool) {
        let priority = match (mode_01, pixel.priority) {
            (_, 1) => 1,
            (true, _) => 4,
            (false, _) => 5,
        };
        self.add(pixel, Layer::Bg1, priority);
    }

    fn add_bg2(&mut self, pixel: Pixel, mode_01: bool) {
        let priority = match (mode_01, pixel.priority) {
            (true, 1) => 2,
            (true, _) => 5,
            (false, 1) => 3,
            (false, _) => 7,
        };
        self.add(pixel, Layer::Bg2, priority);
    }

    fn add_bg3(&mut self, pixel: Pixel, high_priority: bool) {
        if high_priority && pixel.priority == 1 {
            // Over every other layer, whatever was added before
            *self = Self { priority: 0, pixel, layer: Layer::Bg3 };
            return;
        }
        self.add(pixel, Layer::Bg3, if pixel.priority == 1 { 7 } else { 10 });
    }

    fn add_bg4(&mut self, pixel: Pixel) {
        self.add(pixel, Layer::Bg4, if pixel.priority == 1 { 8 } else { 11 });
    }
}

/// The resolved pixels of one screen (main or sub) for a line.
pub(crate) struct ScreenBuffer {
    resolvers: [Resolver; WIDTH],
    pub(crate) rendered: [Rendered; WIDTH],
}

impl ScreenBuffer {
    pub(crate) fn new() -> Self {
        Self { resolvers: [Resolver::NONE; WIDTH], rendered: [Rendered::BACKDROP; WIDTH] }
    }
}

/// Whether `x` is inside window `which` (0 or 1). `x` is a window position: outside the
/// screen's 256 pixels (a wide view's margins) no window is inside.
fn inside(regs: &LineRegs, which: usize, x: i32) -> bool {
    (i32::from(regs.window_left[which])..=i32::from(regs.window_right[which])).contains(&x)
}

/// A pair of window areas combined with a mask logic: `None` when neither window is on.
fn in_windows(regs: &LineRegs, areas: [u8; 2], logic: u8, x: i32) -> bool {
    let window = |which: usize| match areas[which] {
        0 => None,
        1 => Some(inside(regs, which, x)),
        _ => Some(!inside(regs, which, x)),
    };
    match (window(0), window(1)) {
        (Some(a), Some(b)) => match logic {
            0 => a || b,
            1 => a && b,
            2 => a ^ b,
            _ => !(a ^ b),
        },
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => false,
    }
}

fn bg_in_window(regs: &LineRegs, bg: usize, x: i32) -> bool {
    in_windows(
        regs,
        [regs.bg_window_area[0][bg], regs.bg_window_area[1][bg]],
        regs.bg_window_logic[bg],
        x,
    )
}

fn obj_in_window(regs: &LineRegs, x: i32) -> bool {
    in_windows(regs, regs.obj_window_area, regs.obj_window_logic, x)
}

fn in_math_window(regs: &LineRegs, x: i32) -> bool {
    in_windows(regs, regs.math_window_area, regs.math_window_logic, x)
}

/// A color-math enable mode (0 never, 1 outside the color window, 2 inside it, 3 always).
fn enabled(mode: u8, in_window: bool) -> bool {
    match mode {
        0 => false,
        1 => !in_window,
        2 => in_window,
        _ => true,
    }
}

fn color_math_for_any_layer(regs: &LineRegs) -> bool {
    regs.color_math != 0
        && (regs.backdrop_color_math || regs.obj_color_math || regs.bg_color_math.contains(&true))
}

/// Mode 7 and direct color: the color bits become the RGB bits.
pub(crate) fn direct_color(palette: u8, color: u8) -> u16 {
    let color = u16::from(color);
    let palette = u16::from(palette);
    let r = ((color & 0b00_000_111) << 2) | ((palette & 0b001) << 1);
    let g = ((color & 0b00_111_000) << 4) | ((palette & 0b010) << 5);
    let b = ((color & 0b11_000_000) << 7) | ((palette & 0b100) << 10);
    r | g | b
}

/// `round(brightness / 15 * channel * 255 / 31)` for a 5-bit channel: the PPU's table, in
/// integers (the quotient is never exactly one half).
fn channel(value: u16, brightness: u8) -> u8 {
    CHANNELS[usize::from(brightness & 15)][usize::from(value & 31)]
}

/// [`channel`] for every brightness and 5-bit value.
static CHANNELS: [[u8; 32]; 16] = {
    let mut table = [[0_u8; 32]; 16];
    let mut brightness = 0;
    while brightness < 16 {
        let mut value = 0;
        while value < 32 {
            let numerator = brightness as u32 * value as u32 * 17;
            table[brightness][value] = ((2 * numerator + 31) / 62) as u8;
            value += 1;
        }
        brightness += 1;
    }
    table
};

pub(crate) fn convert(color: u16, brightness: u8) -> Color {
    Color::rgb(
        channel(color & 0x1F, brightness),
        channel((color >> 5) & 0x1F, brightness),
        channel((color >> 10) & 0x1F, brightness),
    )
}

/// Color math of the 5-bit channels of `a` and `b`.
fn math(a: u16, b: u16, subtract: bool, divide: bool) -> u16 {
    let mut out = 0;
    for shift in [0, 5, 10] {
        let (a, b) = ((a >> shift) & 0x1F, (b >> shift) & 0x1F);
        let mut value = if subtract { a.saturating_sub(b) } else { a + b };
        value = if divide { value >> 1 } else { value.min(0x1F) };
        out |= value << shift;
    }
    out
}

/// The colour math of one pixel of the main screen: the other screen's colour and how it is
/// combined with the main one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Math {
    pub sub: u16,
    pub subtract: bool,
    pub divide: bool,
}

/// What the forced blanking and the colour math do to the main screen's 15-bit colour of a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plan {
    pub force_black: bool,
    pub math: Option<Math>,
}

impl Plan {
    /// The 15-bit colour the pixel shows, from the main screen's colour `main`.
    pub(crate) fn apply(self, main: u16) -> u16 {
        let main = if self.force_black { 0 } else { main };
        match self.math {
            Some(math_of) => math(main, math_of.sub, math_of.subtract, math_of.divide),
            None => main,
        }
    }
}

impl Renderer {
    /// Draws the pixels of the line from `from` on into `row`.
    pub(crate) fn render_scanline(&mut self, regs: &LineRegs, row: &mut [Color], from: usize) {
        self.resolve_screen(regs, false);
        if regs.sub_screen_layers && color_math_for_any_layer(regs) {
            self.resolve_screen(regs, true);
        }

        for (x, out) in row.iter_mut().enumerate().skip(from) {
            let color = self.plan(regs, x).apply(self.main.rendered[x].color);
            *out = convert(color, regs.brightness);
        }
    }

    /// What the forced blanking and the colour math do to the colour of the main screen at pixel
    /// `x` of the line, after [`Renderer::resolve_screen`]. The Enhanced picture applies the same
    /// plan to its own samples of a Mode 7 plane.
    pub(crate) fn plan(&self, regs: &LineRegs, x: usize) -> Plan {
        let main = self.main.rendered[x];
        self.plan_as(regs, x, main.layer, main.palette)
    }

    /// [`Renderer::plan`] for pixel `x` as if its main screen colour came from `layer` (and an
    /// object's palette row `palette`): what the art that replaces a layer's pixel is shaded with
    /// where another layer or an object is on top in the PPU's own picture.
    pub(crate) fn plan_as(&self, regs: &LineRegs, x: usize, layer: Layer, palette: u8) -> Plan {
        let main = Rendered { palette, layer, ..self.main.rendered[x] };
        let in_color_window = in_math_window(regs, self.origin + x as i32);
        let force_black = enabled(regs.force_main_black, in_color_window);
        let global = enabled(regs.color_math, in_color_window);
        let layer_on = match main.layer {
            Layer::Bg1 => regs.bg_color_math[0],
            Layer::Bg2 => regs.bg_color_math[1],
            Layer::Bg3 => regs.bg_color_math[2],
            Layer::Bg4 => regs.bg_color_math[3],
            Layer::Obj => regs.obj_color_math && main.palette >= 4,
            Layer::Backdrop => regs.backdrop_color_math,
        };
        let math = (global && layer_on).then(|| {
            let (sub_color, sub_transparent) = if regs.sub_screen_layers {
                let pixel = self.sub.rendered[x];
                (pixel.color, pixel.layer == Layer::Backdrop)
            } else {
                (regs.fixed_color, false)
            };
            let divide = regs.color_math_divide && !force_black && !sub_transparent;
            Math { sub: sub_color, subtract: regs.color_math_subtract, divide }
        });
        Plan { force_black, math }
    }

    /// The 15-bit colour of BG `bg` (0 for BG1) at pixel `x` of the line drawn last, when the BG
    /// draws a pixel there (not in mode 7).
    pub(crate) fn bg_colour(&self, regs: &LineRegs, bg: usize, x: usize) -> Option<u16> {
        let pixel = self.bg_pixels[bg][x];
        let layer = match bg {
            0 => Layer::Bg1,
            1 => Layer::Bg2,
            2 => Layer::Bg3,
            _ => Layer::Bg4,
        };
        (!pixel.is_transparent())
            .then(|| resolve_color(&self.cgram, layer, regs.bg_mode, regs.direct_color, pixel))
    }

    /// The CGRAM index of BG `bg` (0 for BG1) at pixel `x` of the line drawn last: the entry
    /// `bg_colour` reads its colour from (not in mode 7 or direct colour).
    pub(crate) fn bg_index(&self, regs: &LineRegs, bg: usize, x: usize) -> Option<u16> {
        let pixel = self.bg_pixels[bg][x];
        let layer = match bg {
            0 => Layer::Bg1,
            1 => Layer::Bg2,
            2 => Layer::Bg3,
            _ => Layer::Bg4,
        };
        if pixel.is_transparent() || regs.bg_mode == 7 || regs.direct_color {
            return None;
        }
        Some(resolve_index(layer, regs.bg_mode, pixel))
    }

    /// Which Mode 7 plane is the top layer of pixel `x`: 1 for BG1, 2 for BG2 (EXTBG), 0 for none.
    pub(crate) fn plane_layer(&self, regs: &LineRegs, x: usize) -> u8 {
        match (regs.bg_mode, self.main.rendered[x].layer) {
            (7, Layer::Bg1) => 1,
            (7, Layer::Bg2) => 2,
            _ => 0,
        }
    }

    /// Resolves the main or the sub screen of the line from the line buffers.
    fn resolve_screen(&mut self, regs: &LineRegs, sub: bool) {
        let (bg_on, bg_off_in_window, obj_on, obj_off_in_window, backdrop) = if sub {
            (
                regs.sub_bg,
                regs.sub_bg_window_off,
                regs.sub_obj,
                regs.sub_obj_window_off,
                regs.fixed_color,
            )
        } else {
            (
                regs.main_bg,
                regs.main_bg_window_off,
                regs.main_obj,
                regs.main_obj_window_off,
                self.cgram[0],
            )
        };
        let mode = regs.bg_mode;
        let mode_01 = mode < 2;
        let origin = self.origin;
        let screen = if sub { &mut self.sub } else { &mut self.main };
        screen.resolvers.fill(Resolver::NONE);

        if obj_on {
            for (x, resolver) in screen.resolvers.iter_mut().enumerate() {
                if obj_off_in_window && obj_in_window(regs, origin + x as i32) {
                    continue;
                }
                let pixel = self.obj_pixels[x];
                if !pixel.is_transparent() {
                    resolver.add_obj(pixel, mode_01);
                }
            }
        }

        if bg_on[0] {
            for (x, resolver) in screen.resolvers.iter_mut().enumerate() {
                if bg_off_in_window[0] && bg_in_window(regs, 0, origin + x as i32) {
                    continue;
                }
                let pixel = self.bg_pixels[0][x];
                if !pixel.is_transparent() {
                    resolver.add_bg1(pixel, mode_01);
                }
            }
        }

        // BG2 shows in modes 0 and 1, and in mode 7 with EXTBG (BG1's pixels, the top bit as
        // priority)
        let bg2_shown = mode < 2 || (mode == 7 && regs.extbg);
        if bg2_shown && bg_on[1] {
            for (x, resolver) in screen.resolvers.iter_mut().enumerate() {
                if bg_off_in_window[1] && bg_in_window(regs, 1, origin + x as i32) {
                    continue;
                }
                if mode == 7 {
                    let source = self.bg_pixels[0][x];
                    let color = source.color & 0x7F;
                    if color != 0 {
                        let pixel = Pixel {
                            palette: 0,
                            color,
                            priority: source.color >> 7,
                            char: 0,
                            pos: 0,
                            oam: crate::NO_OBJECT,
                        };
                        resolver.add_bg2(pixel, false);
                    }
                } else {
                    let pixel = self.bg_pixels[1][x];
                    if !pixel.is_transparent() {
                        resolver.add_bg2(pixel, mode_01);
                    }
                }
            }
        }

        if mode < 2 && bg_on[2] {
            let high = mode == 1 && regs.mode_1_bg3_priority;
            for (x, resolver) in screen.resolvers.iter_mut().enumerate() {
                if bg_off_in_window[2] && bg_in_window(regs, 2, origin + x as i32) {
                    continue;
                }
                let pixel = self.bg_pixels[2][x];
                if !pixel.is_transparent() {
                    resolver.add_bg3(pixel, high);
                }
            }
        }

        if mode == 0 && bg_on[3] {
            for (x, resolver) in screen.resolvers.iter_mut().enumerate() {
                if bg_off_in_window[3] && bg_in_window(regs, 3, origin + x as i32) {
                    continue;
                }
                let pixel = self.bg_pixels[3][x];
                if !pixel.is_transparent() {
                    resolver.add_bg4(pixel);
                }
            }
        }

        for (resolver, rendered) in screen.resolvers.iter().zip(screen.rendered.iter_mut()) {
            *rendered = if resolver.priority == u8::MAX {
                Rendered { color: backdrop, ..Rendered::BACKDROP }
            } else {
                let color = resolve_color(
                    &self.cgram,
                    resolver.layer,
                    mode,
                    regs.direct_color,
                    resolver.pixel,
                );
                Rendered {
                    color,
                    palette: resolver.pixel.palette,
                    layer: resolver.layer,
                    char: resolver.pixel.char,
                    pos: resolver.pixel.pos,
                    oam: resolver.pixel.oam,
                    priority: resolver.priority,
                }
            };
        }
    }

    /// What each pixel of the line from `from` on shows, from the main screen as resolved by
    /// [`Renderer::render_scanline`].
    pub(crate) fn sources_of_line(
        &self,
        regs: &LineRegs,
        binder: &Binder<'_>,
        sources: &mut [Source],
        from: usize,
    ) {
        for (x, source) in sources.iter_mut().enumerate().skip(from) {
            let rendered = self.main.rendered[x];
            *source = match rendered.layer {
                Layer::Backdrop => Source::Backdrop,
                Layer::Obj => Source::Char {
                    object: true,
                    bpp: 4,
                    address: rendered.char,
                    tile: binder.tile(4, rendered.char),
                },
                _ if regs.bg_mode == 7 => Source::Plane,
                layer => {
                    let bg = match layer {
                        Layer::Bg1 => 0,
                        Layer::Bg2 => 1,
                        Layer::Bg3 => 2,
                        _ => 3,
                    };
                    let bpp = bg_bitplanes(regs.bg_mode, bg) as u8;
                    Source::Char {
                        object: false,
                        bpp,
                        address: rendered.char,
                        tile: binder.tile(bpp, rendered.char),
                    }
                }
            };
        }
    }
}

/// The CGRAM index of a layer's pixel (not in mode 7): what [`resolve_color`] looks up.
pub(crate) fn resolve_index(layer: Layer, mode: u8, pixel: Pixel) -> u16 {
    let (palette, color) = (usize::from(pixel.palette), usize::from(pixel.color));
    let index = match layer {
        Layer::Obj => 0x80 | (palette << 4) | color,
        Layer::Bg1 | Layer::Bg2 if mode == 1 => (palette << 4) | color,
        Layer::Bg1 | Layer::Bg2 | Layer::Bg3 | Layer::Bg4 => {
            let offset = if mode == 0 {
                match layer {
                    Layer::Bg2 => 0x20,
                    Layer::Bg3 => 0x40,
                    Layer::Bg4 => 0x60,
                    _ => 0,
                }
            } else {
                0
            };
            offset | (palette << 2) | color
        }
        Layer::Backdrop => 0,
    };
    (index & 255) as u16
}

/// The 15-bit color of a layer's pixel through CGRAM.
pub(crate) fn resolve_color(
    cgram: &[u16; 256],
    layer: Layer,
    mode: u8,
    direct: bool,
    pixel: Pixel,
) -> u16 {
    let (palette, color) = (usize::from(pixel.palette), usize::from(pixel.color));
    let eight_bits = mode == 7 && matches!(layer, Layer::Bg1 | Layer::Bg2);
    if eight_bits {
        return if direct { direct_color(pixel.palette, pixel.color) } else { cgram[color] };
    }
    match layer {
        // Objects: 4 bits from the second half of CGRAM
        Layer::Obj => cgram[0x80 | (palette << 4) | color],
        Layer::Bg1 | Layer::Bg2 if mode == 1 => cgram[(palette << 4) | color],
        // 2 bits: in mode 0 each BG has its own quarter of CGRAM
        Layer::Bg1 | Layer::Bg2 | Layer::Bg3 | Layer::Bg4 => {
            let offset = if mode == 0 {
                match layer {
                    Layer::Bg2 => 0x20,
                    Layer::Bg3 => 0x40,
                    Layer::Bg4 => 0x60,
                    _ => 0,
                }
            } else {
                0
            };
            cgram[offset | (palette << 2) | color]
        }
        Layer::Backdrop => 0,
    }
}
