//! Output-only log of what the PPU rendered in a frame, for a renderer outside the core.
//!
//! Disabled by default. While enabled, the PPU records, for each scanline it renders, the
//! registers the render read (and the sprite parameters of the line buffer it drew from), a copy
//! of CGRAM, VRAM and OAM as the frame starts, and the writes to CGRAM, VRAM and OAM while it is
//! displayed (HDMA gradients, and the forced blanking the game toggles mid-frame). The
//! emulation never reads any of it back, so the machine is bit-identical with the log on or off,
//! and it is not part of save states.

use crate::ppu::registers::{
    ColorMathEnableMode, ColorMathOperation, Mode7OobBehavior, ObjTileSize, Registers,
    WindowAreaMode, WindowMaskLogic,
};
use crate::ppu::{CGRAM_LEN_WORDS, OAM_HIGH_LEN_BYTES, OAM_LOW_LEN_WORDS, VRAM_LEN_WORDS};
use jgenesis_proc_macros::{FakeDecode, FakeEncode};

/// The registers one scanline's render read, as plain values (see the PPU for their meaning).
///
/// Enumerations are numbered as noted; windows' areas are 0 disabled, 1 inside, 2 outside; window
/// mask logic 0 OR, 1 AND, 2 XOR, 3 XNOR; color math enable modes 0 never, 1 outside the color
/// window, 2 inside it, 3 always (`force_main_black`: 0 never, 1 outside, 2 inside, 3 always).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineRegs {
    pub forced_blanking: bool,
    pub brightness: u8,
    /// BGMODE bits 0-2.
    pub bg_mode: u8,
    pub mode_1_bg3_priority: bool,
    pub extbg: bool,
    /// BG character size is 16x16.
    pub bg_large_tiles: [bool; 4],
    /// MOSAIC size (a block is `mosaic_size + 1` pixels).
    pub mosaic_size: u8,
    pub mosaic_bg: [bool; 4],
    /// Lines the mosaic's vertical filter moves up on this line.
    pub mosaic_v_offset: u8,
    /// `BGnSC` size: 0 one screen, 1 64x32, 2 32x64, 3 64x64.
    pub bg_screen_size: [u8; 4],
    /// Tile map base, in VRAM words.
    pub bg_map_base: [u16; 4],
    /// Character base, in VRAM words.
    pub bg_char_base: [u16; 4],
    pub bg_h_scroll: [u16; 4],
    pub bg_v_scroll: [u16; 4],
    pub main_bg: [bool; 4],
    pub main_obj: bool,
    pub sub_bg: [bool; 4],
    pub sub_obj: bool,
    pub main_bg_window_off: [bool; 4],
    pub main_obj_window_off: bool,
    pub sub_bg_window_off: [bool; 4],
    pub sub_obj_window_off: bool,
    pub window_left: [u16; 2],
    pub window_right: [u16; 2],
    /// `[window][bg]`.
    pub bg_window_area: [[u8; 4]; 2],
    pub obj_window_area: [u8; 2],
    pub math_window_area: [u8; 2],
    pub bg_window_logic: [u8; 4],
    pub obj_window_logic: u8,
    pub math_window_logic: u8,
    pub force_main_black: u8,
    pub color_math: u8,
    pub sub_screen_layers: bool,
    pub direct_color: bool,
    pub color_math_subtract: bool,
    pub color_math_divide: bool,
    pub bg_color_math: [bool; 4],
    pub obj_color_math: bool,
    pub backdrop_color_math: bool,
    /// The fixed color (COLDATA), as a 15-bit BGR value.
    pub fixed_color: u16,
    pub m7_h_flip: bool,
    pub m7_v_flip: bool,
    /// 0 wrap, 1 transparent, 2 tile 0.
    pub m7_out_of_bounds: u8,
    pub m7_a: u16,
    pub m7_b: u16,
    pub m7_c: u16,
    pub m7_d: u16,
    pub m7_h_scroll: u16,
    pub m7_v_scroll: u16,
    pub m7_center_x: u16,
    pub m7_center_y: u16,
    pub interlaced: bool,
    pub pseudo_h_hi_res: bool,
    pub pseudo_obj_hi_res: bool,
}

fn area(mode: WindowAreaMode) -> u8 {
    match mode {
        WindowAreaMode::Disabled => 0,
        WindowAreaMode::Inside => 1,
        WindowAreaMode::Outside => 2,
    }
}

fn logic(mode: WindowMaskLogic) -> u8 {
    match mode {
        WindowMaskLogic::Or => 0,
        WindowMaskLogic::And => 1,
        WindowMaskLogic::Xor => 2,
        WindowMaskLogic::Xnor => 3,
    }
}

fn enable_mode(mode: ColorMathEnableMode) -> u8 {
    match mode {
        ColorMathEnableMode::Never => 0,
        ColorMathEnableMode::OutsideColorWindow => 1,
        ColorMathEnableMode::InsideColorWindow => 2,
        ColorMathEnableMode::Always => 3,
    }
}

impl LineRegs {
    pub(super) fn capture(registers: &Registers, mosaic_v_offset: u8) -> Self {
        use crate::ppu::registers::{BgScreenSize, TileSize};

        let m7 = &registers.mode_7_latched;
        Self {
            forced_blanking: registers.forced_blanking,
            brightness: registers.brightness,
            bg_mode: registers.bg_mode as u8,
            mode_1_bg3_priority: registers.mode_1_bg3_priority,
            extbg: registers.extbg_enabled,
            bg_large_tiles: registers.bg_tile_size.map(|size| size == TileSize::Large),
            mosaic_size: registers.mosaic_size,
            mosaic_bg: registers.bg_mosaic_enabled,
            mosaic_v_offset,
            bg_screen_size: registers.bg_screen_size.map(|size| match size {
                BgScreenSize::OneScreen => 0,
                BgScreenSize::VerticalMirror => 1,
                BgScreenSize::HorizontalMirror => 2,
                BgScreenSize::FourScreen => 3,
            }),
            bg_map_base: registers.bg_base_address,
            bg_char_base: registers.bg_tile_base_address,
            bg_h_scroll: registers.bg_h_scroll,
            bg_v_scroll: registers.bg_v_scroll,
            main_bg: registers.main_bg_enabled,
            main_obj: registers.main_obj_enabled,
            sub_bg: registers.sub_bg_enabled,
            sub_obj: registers.sub_obj_enabled,
            main_bg_window_off: registers.main_bg_disabled_in_window,
            main_obj_window_off: registers.main_obj_disabled_in_window,
            sub_bg_window_off: registers.sub_bg_disabled_in_window,
            sub_obj_window_off: registers.sub_obj_disabled_in_window,
            window_left: [registers.window_1_left, registers.window_2_left],
            window_right: [registers.window_1_right, registers.window_2_right],
            bg_window_area: [
                registers.bg_window_1_area.map(area),
                registers.bg_window_2_area.map(area),
            ],
            obj_window_area: [area(registers.obj_window_1_area), area(registers.obj_window_2_area)],
            math_window_area: [
                area(registers.math_window_1_area),
                area(registers.math_window_2_area),
            ],
            bg_window_logic: registers.bg_window_mask_logic.map(logic),
            obj_window_logic: logic(registers.obj_window_mask_logic),
            math_window_logic: logic(registers.math_window_mask_logic),
            force_main_black: enable_mode(registers.force_main_screen_black),
            color_math: enable_mode(registers.color_math_enabled),
            sub_screen_layers: registers.sub_bg_obj_enabled,
            direct_color: registers.direct_color_mode_enabled,
            color_math_subtract: registers.color_math_operation == ColorMathOperation::Subtract,
            color_math_divide: registers.color_math_divide_enabled,
            bg_color_math: registers.bg_color_math_enabled,
            obj_color_math: registers.obj_color_math_enabled,
            backdrop_color_math: registers.backdrop_color_math_enabled,
            fixed_color: registers.sub_backdrop_color,
            m7_h_flip: m7.h_flip,
            m7_v_flip: m7.v_flip,
            m7_out_of_bounds: match m7.oob_behavior {
                Mode7OobBehavior::Wrap => 0,
                Mode7OobBehavior::Transparent => 1,
                Mode7OobBehavior::Tile0 => 2,
            },
            m7_a: m7.parameter_a,
            m7_b: m7.parameter_b,
            m7_c: m7.parameter_c,
            m7_d: m7.parameter_d,
            m7_h_scroll: m7.h_scroll,
            m7_v_scroll: m7.v_scroll,
            m7_center_x: m7.center_x,
            m7_center_y: m7.center_y,
            interlaced: registers.interlaced,
            pseudo_h_hi_res: registers.pseudo_h_hi_res,
            pseudo_obj_hi_res: registers.pseudo_obj_hi_res,
        }
    }
}

/// How the sprite line buffer a scanline drew from was made: sprites are evaluated at the end of
/// the previous scanline's display (dot 256) and their tiles fetched at its end, with the object
/// registers as they were at each point (a per-line HDMA of OBSEL changes the second but not the
/// first).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpriteLine {
    /// The scanline the sprites were evaluated for: their row is `eval_line - y`, and they are
    /// drawn in the next scanline.
    pub eval_line: u16,
    /// No evaluation ran (forced blanking or vertical blank at the start of the line).
    pub blank: bool,
    /// The first OAM index of the evaluation (0, or the OAM address when priority rotation is on).
    pub start_oam_idx: u8,
    /// OBSEL's size field (0-7) when the sprites were evaluated.
    pub scan_size: u8,
    /// OBSEL's size field, name base (VRAM words) and name gap (VRAM words) when the tiles were
    /// fetched.
    pub fetch_size: u8,
    pub fetch_base: u16,
    pub fetch_gap: u16,
    /// Forced blanking when the tiles were fetched: they are then all transparent.
    pub fetch_blanked: bool,
    /// The line buffer was rebuilt at the end of the scanline (it is skipped, and keeps the
    /// previous scanline's sprites, when no object layer is enabled then).
    pub rebuilt: bool,
}

impl SpriteLine {
    pub(super) fn obj_registers(registers: &Registers) -> (u8, u16, u16) {
        (
            obj_size_index(registers.obj_tile_size),
            registers.obj_tile_base_address,
            registers.obj_tile_gap_size,
        )
    }
}

fn obj_size_index(size: ObjTileSize) -> u8 {
    match size {
        ObjTileSize::Zero => 0,
        ObjTileSize::One => 1,
        ObjTileSize::Two => 2,
        ObjTileSize::Three => 3,
        ObjTileSize::Four => 4,
        ObjTileSize::Five => 5,
        ObjTileSize::Six => 6,
        ObjTileSize::Seven => 7,
    }
}

/// One render of a scanline. A write to a scroll register or INIDISP while the line is drawn
/// renders it again from `from_pixel` on, with the registers as they are then: a line has the
/// entry of its start (`from_pixel` 0) and then those.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineEntry {
    /// 1-based: line `n` is row `n - 1` of the frame.
    pub scanline: u16,
    pub from_pixel: u16,
    pub regs: LineRegs,
    pub sprites: SpriteLine,
}

/// A write to CGRAM in the frame, applied to the entries from number `entry` on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CgramWrite {
    pub entry: u16,
    pub index: u8,
    pub value: u16,
}

/// A write to VRAM while the frame is displayed (the display is in forced blanking), applied to the
/// entries from number `entry` on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VramWrite {
    pub entry: u16,
    /// Word address.
    pub address: u16,
    pub value: u16,
}

/// A write to OAM while the frame is displayed, applied to the entries from number `entry` on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OamWrite {
    pub entry: u16,
    /// A byte of the high table (`index` 0-31, `value` the byte) or a word of the low table
    /// (`index` 0-255, `value` the word).
    pub high: bool,
    pub index: u16,
    pub value: u16,
}

/// What the PPU rendered in a frame (see the module documentation).
#[derive(Debug, Clone)]
pub struct FrameLog {
    /// The frame was logged from its first scanline to the end of its display.
    pub complete: bool,
    /// Rows of the frame (224 or 239; doubled for an interlaced frame).
    pub height: u16,
    /// A mode 5 or 6, or pseudo-hi-res, line was rendered in the frame (the frame is 512 wide).
    pub h_hi_res: bool,
    /// The frame is interlaced (it has twice the rows).
    pub v_hi_res: bool,
    pub entries: Vec<LineEntry>,
    /// CGRAM at the start of the frame.
    pub cgram_start: Vec<u16>,
    pub cgram_writes: Vec<CgramWrite>,
    /// VRAM and OAM (low table as bytes, high table) at the start of the frame (the display
    /// reads them as they were then, but for `vram_writes`).
    pub vram: Vec<u16>,
    pub vram_writes: Vec<VramWrite>,
    pub oam_low: Vec<u8>,
    pub oam_high: Vec<u8>,
    pub oam_writes: Vec<OamWrite>,
}

impl FrameLog {
    fn new() -> Self {
        Self {
            complete: false,
            height: 0,
            h_hi_res: false,
            v_hi_res: false,
            entries: Vec::with_capacity(256),
            cgram_start: vec![0; CGRAM_LEN_WORDS],
            cgram_writes: Vec::new(),
            vram: vec![0; VRAM_LEN_WORDS],
            vram_writes: Vec::new(),
            oam_low: vec![0; OAM_LOW_LEN_WORDS * 2],
            oam_high: vec![0; OAM_HIGH_LEN_BYTES],
            oam_writes: Vec::new(),
        }
    }
}

/// The memories the display reads, as the frame starts.
#[derive(Clone, Copy)]
pub(super) struct Memories<'a> {
    pub cgram: &'a [u16],
    pub vram: &'a [u16],
    pub oam_low: &'a [u8],
    pub oam_high: &'a [u8],
}

/// The recorder inside the PPU: not encoded, not compared; its setting is carried over by the
/// owner of the PPU.
#[derive(Debug, Clone, FakeEncode, FakeDecode)]
pub(super) struct LineLog {
    pub enabled: bool,
    /// The frame in progress is being logged from its start.
    pub recording: bool,
    pub log: FrameLog,
    /// Sprite parameters of the scanline being evaluated.
    pub collecting: SpriteLine,
    /// Those of the scanline whose sprites the next render draws.
    pub ready: SpriteLine,
}

impl Default for LineLog {
    fn default() -> Self {
        Self {
            enabled: false,
            recording: false,
            log: FrameLog::new(),
            collecting: SpriteLine { blank: true, ..SpriteLine::default() },
            ready: SpriteLine { blank: true, ..SpriteLine::default() },
        }
    }
}

impl LineLog {
    /// The same setting, without the logged frame.
    pub fn clone_setting(&self) -> Self {
        Self { enabled: self.enabled, ..Self::default() }
    }

    /// A frame starts: the memories are copied as the display will read them.
    pub fn begin_frame(&mut self, memories: Memories<'_>) {
        self.recording = self.enabled;
        if !self.enabled {
            return;
        }
        let log = &mut self.log;
        log.complete = false;
        log.h_hi_res = false;
        log.v_hi_res = false;
        log.entries.clear();
        log.cgram_writes.clear();
        log.vram_writes.clear();
        log.oam_writes.clear();
        log.cgram_start.copy_from_slice(memories.cgram);
        log.vram.copy_from_slice(memories.vram);
        log.oam_low.copy_from_slice(memories.oam_low);
        log.oam_high.copy_from_slice(memories.oam_high);
    }

    pub fn finish_frame(&mut self, height: u16, h_hi_res: bool, v_hi_res: bool) {
        if !self.recording {
            return;
        }
        let log = &mut self.log;
        log.height = height;
        log.h_hi_res = h_hi_res;
        log.v_hi_res = v_hi_res;
        log.complete = true;
        self.recording = false;
    }

    pub fn push_entry(&mut self, scanline: u16, from_pixel: u16, regs: LineRegs) {
        if self.recording {
            self.log.entries.push(LineEntry { scanline, from_pixel, regs, sprites: self.ready });
        }
    }

    pub fn push_vram_write(&mut self, address: u16, value: u16) {
        if self.recording {
            let entry = self.log.entries.len() as u16;
            self.log.vram_writes.push(VramWrite { entry, address, value });
        }
    }

    pub fn push_oam_write(&mut self, high: bool, index: u16, value: u16) {
        if self.recording {
            let entry = self.log.entries.len() as u16;
            self.log.oam_writes.push(OamWrite { entry, high, index, value });
        }
    }

    pub fn push_cgram_write(&mut self, index: u8, value: u16) {
        if self.recording {
            let entry = self.log.entries.len() as u16;
            self.log.cgram_writes.push(CgramWrite { entry, index, value });
        }
    }
}
