//! SNES public interface and main loop

use crate::apu::{Apu, ApuTickEffect};
use crate::audio::AudioResampler;
use crate::bus::Bus;
use crate::input::SnesInputs;
use crate::memory::cartridge::RomImage;
use crate::memory::dma::{DmaStatus, DmaUnit};
use crate::memory::{CpuInternalRegisters, Memory};
use crate::ppu::linelog::FrameLog;
use crate::ppu::{Ppu, PpuTickEffect};
use crate::{bus, ppu};
use bincode::error::EncodeError;
use bincode::{Decode, Encode};
use crc::Crc;
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorConfigTrait, EmulatorTrait, InputPoller, NtscPerFrameParams,
    PartialClone, RenderFrameOptions, Renderer, SaveWriter, TickEffect, TimingMode,
};
use jgenesis_proc_macros::{ConfigDisplay, FakeDecode, FakeEncode};
use snes_config::{AudioInterpolationMode, SnesAspectRatio, SnesButton};
use snes_coprocessors::st018::St018LoadError;
use std::fmt::{Debug, Display};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::{io, mem};
use thiserror::Error;
use wdc65816_emu::core::{Registers, Wdc65816};
use wdc65816_emu::traits::BusInterface;

const MEMORY_REFRESH_MCLK: u64 = 536;
const MEMORY_REFRESH_CYCLES: u64 = 40;

const CRC: Crc<u32> = Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);

#[derive(Debug, Clone, Copy, Encode, Decode, ConfigDisplay)]
pub struct SnesEmulatorConfig {
    pub forced_timing_mode: Option<TimingMode>,
    pub aspect_ratio: SnesAspectRatio,
    pub deinterlace: bool,
    pub audio_interpolation: AudioInterpolationMode,
    pub audio_60hz_hack: bool,
    pub gsu_overclock_factor: NonZeroU64,
    pub allow_opposing_joypad_directions: bool,
    pub mouse_sensitivity: f64,
    /// Seed for a deterministic power-on WRAM fill; `None` fills with `rand::random` (upstream).
    pub wram_fill_seed: Option<u64>,
}

impl EmulatorConfigTrait for SnesEmulatorConfig {
    fn with_overclocking_disabled(&self) -> Self {
        Self { gsu_overclock_factor: NonZeroU64::new(1).unwrap(), ..*self }
    }
}

impl Default for SnesEmulatorConfig {
    fn default() -> Self {
        Self {
            forced_timing_mode: None,
            aspect_ratio: SnesAspectRatio::default(),
            deinterlace: true,
            audio_interpolation: AudioInterpolationMode::default(),
            audio_60hz_hack: false,
            gsu_overclock_factor: NonZeroU64::new(1).unwrap(),
            allow_opposing_joypad_directions: false,
            mouse_sensitivity: 1.0,
            wram_fill_seed: None,
        }
    }
}

pub type CoprocessorRomFn =
    dyn Fn() -> Result<Vec<u8>, (io::Error, String)> + Send + Sync + 'static;

#[derive(Default, FakeEncode, FakeDecode)]
pub struct CoprocessorRoms {
    pub dsp1: Option<Box<CoprocessorRomFn>>,
    pub dsp2: Option<Box<CoprocessorRomFn>>,
    pub dsp3: Option<Box<CoprocessorRomFn>>,
    pub dsp4: Option<Box<CoprocessorRomFn>>,
    pub st010: Option<Box<CoprocessorRomFn>>,
    pub st011: Option<Box<CoprocessorRomFn>>,
    pub st018: Option<Box<CoprocessorRomFn>>,
}

impl CoprocessorRoms {
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }
}

#[derive(Debug, Error)]
pub enum SnesError<RErr, AErr, SErr> {
    #[error("Error rendering frame: {0}")]
    Render(RErr),
    #[error("Error outputting audio samples: {0}")]
    AudioOutput(AErr),
    #[error("Error persisting save file: {0}")]
    SaveWrite(SErr),
    #[error("Error encoding save file bytes: {0}")]
    SaveEncode(#[from] EncodeError),
}

#[derive(Debug, Error)]
pub enum SnesLoadError {
    #[error("ROM is only {size} bytes; all SNES ROM chips should be at least 32768 bytes")]
    RomTooSmall { size: usize },
    #[error("Cannot load DSP-1 cartridge because DSP-1 ROM is not configured")]
    MissingDsp1Rom,
    #[error("Cannot load DSP-2 cartridge because DSP-2 ROM is not configured")]
    MissingDsp2Rom,
    #[error("Cannot load DSP-3 cartridge because DSP-3 ROM is not configured")]
    MissingDsp3Rom,
    #[error("Cannot load DSP-4 cartridge because DSP-4 ROM is not configured")]
    MissingDsp4Rom,
    #[error("Cannot load ST010 cartridge because ST010 ROM is not configured")]
    MissingSt010Rom,
    #[error("Cannot load ST011 cartridge because ST011 ROM is not configured")]
    MissingSt011Rom,
    #[error("Cannot load ST018 cartridge because ST018 ROM is not configured")]
    MissingSt018Rom,
    #[error("Failed to load required coprocessor ROM from '{path}': {source}")]
    CoprocessorRomLoad {
        #[source]
        source: io::Error,
        path: String,
    },
    #[error("Invalid ST018 coprocessor ROM: {0}")]
    St018RomLoad(St018LoadError),
}

pub type SnesLoadResult<T> = Result<T, SnesLoadError>;

macro_rules! new_bus {
    ($self:expr) => {
        Bus {
            memory: &mut $self.memory,
            cpu_registers: &mut $self.cpu_registers,
            ppu: &mut $self.ppu,
            apu: &mut $self.apu,
            latched_interrupts: $self.latched_interrupts,
            access_master_cycles: 0,
            pending_write: None,
        }
    };
}

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub(crate) struct LatchedInterrupts {
    pub nmi: bool,
    pub irq: bool,
}

/// CPU writes recorded for an external observer while enabled (not part of save states)
#[derive(Debug, Clone, Default, FakeEncode, FakeDecode)]
struct CpuWriteLog {
    enabled: bool,
    writes: Vec<(u32, u8)>,
}

/// Instrumentation counters (not part of save states)
#[derive(Debug, Clone, Copy, Default, FakeEncode, FakeDecode)]
struct CpuCounters {
    busy_master_cycles: u64,
    opcode_fetches: u64,
}

/// Set of hooked code addresses: one bit per physical key `(PBR << 16 | PC) & $7FFFFF` (2^23
/// bits). Every method takes a 24-bit `PBR << 16 | PC` and ignores bank bit 7, which only
/// selects a mirror.
#[derive(Clone)]
pub struct HookBitmap {
    words: Box<[u64]>,
}

impl HookBitmap {
    /// Mask that turns a 24-bit `PBR << 16 | PC` into a physical key
    pub const KEY_MASK: u32 = 0x7F_FFFF;

    #[must_use]
    pub fn new() -> Self {
        Self { words: vec![0; (Self::KEY_MASK as usize + 1) / 64].into_boxed_slice() }
    }

    /// Physical key of a 24-bit `PBR << 16 | PC`
    #[inline]
    #[must_use]
    pub const fn key(address: u32) -> u32 {
        address & Self::KEY_MASK
    }

    pub fn set(&mut self, address: u32) {
        let (word, bit) = Self::locate(address);
        self.words[word] |= bit;
    }

    pub fn clear(&mut self, address: u32) {
        let (word, bit) = Self::locate(address);
        self.words[word] &= !bit;
    }

    #[inline]
    #[must_use]
    pub fn contains(&self, address: u32) -> bool {
        let (word, bit) = Self::locate(address);
        self.words[word] & bit != 0
    }

    #[inline]
    fn locate(address: u32) -> (usize, u64) {
        let key = Self::key(address) as usize;
        (key / 64, 1 << (key % 64))
    }
}

impl Default for HookBitmap {
    fn default() -> Self {
        Self::new()
    }
}

/// Installed hook bitmap and the hit not taken yet (not part of save states)
#[derive(Default, FakeEncode, FakeDecode)]
struct Hooks {
    bitmap: Option<Arc<HookBitmap>>,
    hit: Option<u32>,
}

#[derive(Encode, Decode, PartialClone)]
pub struct SnesEmulator {
    main_cpu: Wdc65816,
    cpu_registers: CpuInternalRegisters,
    dma_unit: DmaUnit,
    #[partial_clone(partial)]
    memory: Memory,
    ppu: Ppu,
    apu: Apu,
    audio_resampler: AudioResampler,
    total_master_cycles: u64,
    latched_interrupts: Option<LatchedInterrupts>,
    memory_refresh_pending: bool,
    timing_mode: TimingMode,
    aspect_ratio: SnesAspectRatio,
    frame_count: u64,
    last_sram_checksum: u32,
    // Following fields only stored here to enable hard reset
    #[partial_clone(default)]
    coprocessor_roms: CoprocessorRoms,
    emulator_config: SnesEmulatorConfig,
    // Instrumentation, not encoded: carried over by `load_state`
    #[partial_clone(default)]
    cpu_write_log: CpuWriteLog,
    #[partial_clone(default)]
    cpu_counters: CpuCounters,
    #[partial_clone(default)]
    hooks: Hooks,
}

impl SnesEmulator {
    /// # Errors
    ///
    /// This function will return an error if it is unable to load the cartridge ROM for any reason.
    pub fn create<S: SaveWriter>(
        rom: Vec<u8>,
        config: SnesEmulatorConfig,
        coprocessor_roms: CoprocessorRoms,
        save_writer: &mut S,
    ) -> SnesLoadResult<Self> {
        Self::create_from_image(rom, config, coprocessor_roms, save_writer)
    }

    fn create_from_image<S: SaveWriter>(
        rom: impl Into<RomImage>,
        config: SnesEmulatorConfig,
        coprocessor_roms: CoprocessorRoms,
        save_writer: &mut S,
    ) -> SnesLoadResult<Self> {
        let main_cpu = Wdc65816::new();
        let cpu_registers = CpuInternalRegisters::new(&config);
        let dma_unit = DmaUnit::new();

        let initial_sram = save_writer.load_bytes("sav").ok();
        let sram_checksum = initial_sram.as_ref().map_or(0, |sram| CRC.checksum(sram));
        let memory = Memory::create(
            rom,
            initial_sram,
            &coprocessor_roms,
            config.forced_timing_mode,
            config.gsu_overclock_factor,
            config.wram_fill_seed,
            save_writer,
        )?;

        let timing_mode =
            config.forced_timing_mode.unwrap_or_else(|| memory.cartridge_timing_mode());
        let ppu = Ppu::new(timing_mode, config);
        let apu = Apu::new(timing_mode, config);

        log::info!("Running with timing/display mode {timing_mode}");

        let mut emulator = Self {
            main_cpu,
            cpu_registers,
            dma_unit,
            memory,
            ppu,
            apu,
            audio_resampler: AudioResampler::new(),
            total_master_cycles: 0,
            latched_interrupts: None,
            memory_refresh_pending: false,
            timing_mode,
            aspect_ratio: config.aspect_ratio,
            frame_count: 0,
            last_sram_checksum: sram_checksum,
            coprocessor_roms,
            emulator_config: config,
            cpu_write_log: CpuWriteLog::default(),
            cpu_counters: CpuCounters::default(),
            hooks: Hooks::default(),
        };

        // Reset CPU so that execution starts from the right place
        emulator.main_cpu.reset(&mut new_bus!(emulator));

        Ok(emulator)
    }

    #[must_use]
    pub fn cartridge_title(&mut self) -> String {
        self.memory.cartridge_title()
    }

    #[inline]
    #[must_use]
    pub fn has_sram(&self) -> bool {
        self.memory.has_battery_backed_sram()
    }

    /// Cartridge SRAM, if the cartridge has any.
    #[must_use]
    pub fn peek_sram(&self) -> Option<&[u8]> {
        self.memory.sram()
    }

    /// Runnable copy of the emulator that shares the ROM with `self`.
    ///
    /// Unlike `to_save_state()`, the PPU frame buffer is not copied (it starts zeroed). Only plain
    /// LoROM/HiROM cartridges share their ROM; for other mappers the copy has an empty ROM and
    /// must not be run. Coprocessor ROM loaders are not carried over, so the copy cannot be hard
    /// reset into a coprocessor cartridge. The copy keeps the CPU-write log setting and the
    /// counters (the HDMA copy-unit and NMI counters included), and starts with an empty log. It
    /// keeps the hook bitmap; a pending hook hit is not copied.
    #[must_use]
    pub fn snapshot(&self) -> Self {
        // No `..` base: a field added later must be handled here explicitly
        Self {
            main_cpu: self.main_cpu.clone(),
            // The clone keeps the NMI counter
            cpu_registers: self.cpu_registers.clone(),
            // The clone keeps the HDMA copy-unit counter
            dma_unit: self.dma_unit.clone(),
            memory: self.memory.partial_clone(),
            ppu: self.ppu.clone_without_frame_buffer(),
            apu: self.apu.clone(),
            audio_resampler: self.audio_resampler.clone(),
            total_master_cycles: self.total_master_cycles,
            latched_interrupts: self.latched_interrupts,
            memory_refresh_pending: self.memory_refresh_pending,
            timing_mode: self.timing_mode,
            aspect_ratio: self.aspect_ratio,
            frame_count: self.frame_count,
            last_sram_checksum: self.last_sram_checksum,
            coprocessor_roms: CoprocessorRoms::none(),
            emulator_config: self.emulator_config,
            cpu_write_log: CpuWriteLog { enabled: self.cpu_write_log.enabled, writes: Vec::new() },
            cpu_counters: self.cpu_counters,
            hooks: Hooks { bitmap: self.hooks.bitmap.clone(), hit: None },
        }
    }

    #[must_use]
    pub fn cpu_registers(&self) -> Registers {
        self.main_cpu.registers().clone()
    }

    pub fn set_cpu_registers(&mut self, registers: Registers) {
        self.main_cpu.set_registers(registers);
    }

    /// Whether the CPU is between two cycles of an instruction or of an interrupt entry sequence
    #[must_use]
    pub fn is_mid_instruction(&self) -> bool {
        self.main_cpu.is_mid_instruction()
    }

    /// Whether the CPU is in a hardware interrupt entry sequence (NMI or IRQ): from the tick that
    /// starts it to the tick that loads the vector. BRK and COP are instructions and do not count
    #[must_use]
    pub fn is_handling_interrupt(&self) -> bool {
        self.main_cpu.is_handling_interrupt()
    }

    /// Reads `address` as the CPU would, with the same side effects (e.g. read-to-clear registers)
    pub fn bus_read(&mut self, address: u32) -> u8 {
        new_bus!(self).read(address)
    }

    /// Writes `address` as the CPU would, immediately; recorded in the CPU-write log if enabled
    pub fn bus_write(&mut self, address: u32, value: u8) {
        self.apply_cpu_write(address, value);
    }

    #[inline]
    fn apply_cpu_write(&mut self, address: u32, value: u8) {
        if self.cpu_write_log.enabled {
            self.cpu_write_log.writes.push((address, value));
        }
        new_bus!(self).apply_write(address, value);
    }

    /// Records CPU writes (from the CPU and `bus_write`, never DMA) while enabled
    pub fn set_cpu_write_log(&mut self, enabled: bool) {
        self.cpu_write_log.enabled = enabled;
    }

    /// CPU writes recorded since the last call, in order, as (address, value)
    pub fn take_cpu_writes(&mut self) -> Vec<(u32, u8)> {
        mem::take(&mut self.cpu_write_log.writes)
    }

    /// Installs the hook bitmap (`None` removes it). A pending hit is kept if the new bitmap
    /// still contains its address, else dropped. While a bitmap is installed, every `tick` that
    /// leaves the CPU at an opcode fetch (`at_fetch_boundary`) from an address in the bitmap
    /// records a hit; ticks of `advance_idle` hold the CPU and record nothing.
    pub fn set_hooks(&mut self, hooks: Option<Arc<HookBitmap>>) {
        let hit = self
            .hooks
            .hit
            .filter(|&address| hooks.as_ref().is_some_and(|bitmap| bitmap.contains(address)));
        self.hooks = Hooks { bitmap: hooks, hit };
    }

    /// Records a hit if the CPU is at an opcode fetch (`at_fetch_boundary`) from an address in
    /// the installed bitmap: the check that ends every tick that does not hold the CPU. Call it
    /// after moving the CPU without such a tick (e.g. a return performed through
    /// `set_cpu_registers`), or an arrival at a hooked address is not reported
    #[inline(always)]
    pub fn detect_hook_hit(&mut self) {
        if let Some(bitmap) = &self.hooks.bitmap
            && self.at_fetch_boundary()
        {
            let registers = self.main_cpu.registers();
            let address = (u32::from(registers.pbr) << 16) | u32::from(registers.pc);
            if bitmap.contains(address) {
                self.hooks.hit = Some(address);
            }
        }
    }

    /// Samples the NMI and IRQ lines into the CPU, as the final cycle of every instruction does.
    /// Ticks that hold the CPU (`advance_idle`) do not poll: after a hold that stands in for
    /// executed code, call this so an NMI raised during the hold is taken before the next opcode
    /// fetch, as it would be after the code's last instruction
    pub fn poll_cpu_interrupt_lines(&mut self) {
        self.main_cpu.poll_interrupt_lines(&mut new_bus!(self));
    }

    /// The hit recorded by the last `tick` (or by `detect_hook_hit`), as the 24-bit
    /// `PBR << 16 | PC`, if not taken yet.
    /// The next `tick` fetches the opcode there, after which the CPU is mid-instruction, so a
    /// hooked address reports one hit per arrival
    pub fn take_hook_hit(&mut self) -> Option<u32> {
        self.hooks.hit.take()
    }

    /// Starts or stops the PPU's log of what each frame's render read (see
    /// `ppu::linelog`): output only, the emulation is identical either way. The setting
    /// survives a state load and a hard reset.
    pub fn set_ppu_line_log(&mut self, enabled: bool) {
        self.ppu.set_line_log(enabled);
    }

    /// Whether the PPU's render log is on (see `set_ppu_line_log`).
    #[must_use]
    pub fn ppu_line_log_enabled(&self) -> bool {
        self.ppu.line_log_enabled()
    }

    /// Turns the drawing of the PPU's pixels on or off (see `Ppu::set_compose_pixels`): with it
    /// off the CPU, the APU, DMA and every flag and counter run exactly as with it on, only the
    /// frame buffer is not drawn. On by default; the setting survives a state load and a hard
    /// reset.
    pub fn set_ppu_compose_pixels(&mut self, on: bool) {
        self.ppu.set_compose_pixels(on);
    }

    /// Turns the resampling of the audio on or off. With it off the APU and the DSP run as
    /// ever (the CPU reads the same ports) but no sample is filtered, resampled or output; when
    /// it is turned on again the resampler continues from the samples it had. On by default; the
    /// setting survives a state load and a hard reset.
    pub fn set_audio_resampling(&mut self, on: bool) {
        self.audio_resampler.set_enabled(on);
    }

    /// The log of the last frame the PPU completed, if the log is on and the frame was logged
    /// from its start.
    #[must_use]
    pub fn ppu_frame_log(&self) -> Option<&FrameLog> {
        self.ppu.frame_log()
    }

    #[must_use]
    pub fn peek_wram(&self) -> &[u8] {
        self.memory.wram()
    }

    #[must_use]
    pub fn peek_vram(&self) -> &[u16] {
        self.ppu.vram()
    }

    #[must_use]
    pub fn peek_cgram(&self) -> &[u16] {
        self.ppu.cgram()
    }

    /// OAM as (low table, high table); the low table's words are viewed as bytes (native endian)
    #[must_use]
    pub fn peek_oam(&self) -> (&[u8], &[u8]) {
        self.ppu.oam()
    }

    #[must_use]
    pub fn total_master_cycles(&self) -> u64 {
        self.total_master_cycles
    }

    /// Master cycles spent on CPU cycles, i.e. everything except DMA and memory refresh (CPU
    /// cycles held by `advance_idle` included); not part of save states
    #[must_use]
    pub fn cpu_busy_cycles(&self) -> u64 {
        self.cpu_counters.busy_master_cycles
    }

    /// Number of instructions the CPU has started (opcode fetches); not part of save states
    #[must_use]
    pub fn opcode_fetches(&self) -> u64 {
        self.cpu_counters.opcode_fetches
    }

    /// Whether HDMA is pending, reloading or transferring
    #[must_use]
    pub fn hdma_busy(&self) -> bool {
        self.dma_unit.hdma_busy()
    }

    /// Number of HDMA copy units performed (one per active channel on each scanline that
    /// transfers data); a sticky counter, not part of save states: use differences
    #[must_use]
    pub fn hdma_copy_units(&self) -> u64 {
        self.dma_unit.hdma_copy_units()
    }

    /// Number of NMIs the CPU has taken (one per NMI entry, nested ones included); a sticky
    /// counter, not part of save states: use differences
    #[must_use]
    pub fn nmi_count(&self) -> u64 {
        self.cpu_registers.nmi_count()
    }

    /// Master cycles from now until the next timing event the CPU's own code does not start: the
    /// vblank NMI edge (if NMI is enabled in NMITIMEN), the next H/V IRQ (if enabled and the
    /// CPU's I flag is clear) and HDMA taking the bus (the frame-start reload if a channel is
    /// enabled in HDMAEN, the per-line transfers if one is active); 0 if an NMI or IRQ is pending
    /// (a cartridge IRQ included) or HDMA is busy or due on the next tick. No such event comes
    /// sooner (positions as `Ppu::master_cycles_until` gives them); `u64::MAX` if none can come.
    /// GPDMA, which the CPU starts, and future cartridge IRQs are not predicted
    #[must_use]
    pub fn master_cycles_until_timing_event(&self) -> u64 {
        const IRQ_DISABLE_FLAG: u8 = 0x04;

        let irq_enabled = u8::from(self.main_cpu.registers().p) & IRQ_DISABLE_FLAG == 0;
        if irq_enabled && self.memory.cartridge_irq() {
            return 0;
        }
        let interrupt = self.cpu_registers.master_cycles_until_interrupt(&self.ppu, irq_enabled);
        let hdma = self.dma_unit.master_cycles_until_hdma(&self.ppu, &self.cpu_registers);
        interrupt.min(hdma)
    }

    /// Whether bytes of a GPDMA requested through $420B remain to be copied. The channel flags
    /// clear one DMA tick before the DMA unit is idle: with the CPU held at an opcode fetch, the
    /// transfer is over once this is false and `at_fetch_boundary` is true again
    #[must_use]
    pub fn gpdma_requested(&self) -> bool {
        self.cpu_registers.gpdma_requested()
    }

    /// Whether the CPU is at an opcode fetch: no memory refresh pending, no DMA cycle to come
    /// (`DmaUnit::lets_cpu_run`: DMA idle, or pending with nothing to transfer, like the
    /// frame-start HDMA reload check with no channel enabled), and the CPU is between
    /// instructions with no interrupt to handle first. When true, the next tick that does not
    /// hold the CPU is the opcode fetch: HDMA or GPDMA that becomes pending on that tick starts
    /// on a later one.
    #[must_use]
    pub fn at_fetch_boundary(&self) -> bool {
        !self.memory_refresh_pending
            && self.main_cpu.will_fetch_opcode()
            && self.dma_unit.lets_cpu_run(&self.cpu_registers)
    }

    /// Ticks with the CPU held (the rest of the system runs) until at least `master_cycles` have
    /// elapsed; returns whether a frame was rendered
    ///
    /// Held ticks record no hook hit and do not poll the interrupt lines. To let the CPU resume a
    /// hooked opcode fetch after a hold, hold until `at_fetch_boundary()` is true: otherwise the
    /// unheld tick that completes a pending refresh or DMA ends at the same fetch and records
    /// the arrival a second time.
    ///
    /// # Errors
    ///
    /// Propagates any error encountered while rendering frames, pushing audio samples, or
    /// persisting save files.
    #[allow(clippy::type_complexity)]
    pub fn advance_idle<R, A, I, S>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        input_poller: &mut I,
        save_writer: &mut S,
        master_cycles: u64,
    ) -> Result<bool, SnesError<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        I: InputPoller<SnesInputs>,
        S: SaveWriter,
    {
        let start = self.total_master_cycles;
        let mut frame_rendered = false;
        while self.total_master_cycles - start < master_cycles {
            let tick_effect =
                self.tick_inner(renderer, audio_output, input_poller, save_writer, true, false)?;
            frame_rendered |= tick_effect == TickEffect::FrameRendered;
        }

        Ok(frame_rendered)
    }

    /// One tick with the CPU held, as in `advance_idle`, except that if the tick is a CPU cycle
    /// (no memory refresh or DMA) it lasts 8 master cycles (a slow memory access) when `slow`,
    /// else 6 (fast, as in `advance_idle`). Held cycles of 6 and 8 add up to every even number of
    /// master cycles from 12 up, so a caller can hold the CPU for an exact number of CPU-busy
    /// master cycles (`cpu_busy_cycles`). Returns whether a frame was rendered
    ///
    /// # Errors
    ///
    /// Propagates any error encountered while rendering frames, pushing audio samples, or
    /// persisting save files.
    #[allow(clippy::type_complexity)]
    pub fn hold_tick<R, A, I, S>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        input_poller: &mut I,
        save_writer: &mut S,
        slow: bool,
    ) -> Result<bool, SnesError<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        I: InputPoller<SnesInputs>,
        S: SaveWriter,
    {
        let tick_effect =
            self.tick_inner(renderer, audio_output, input_poller, save_writer, true, slow)?;
        Ok(tick_effect == TickEffect::FrameRendered)
    }

    /// Persist SRAM now if it changed since the last write, without waiting for the periodic
    /// check in `tick` (e.g. before the frontend exits)
    ///
    /// # Errors
    ///
    /// Propagates any error returned by the save writer.
    pub fn flush_sram<S: SaveWriter>(&mut self, save_writer: &mut S) -> Result<(), S::Err> {
        self.persist_sram_if_changed(save_writer)
    }

    fn persist_sram_if_changed<S: SaveWriter>(
        &mut self,
        save_writer: &mut S,
    ) -> Result<(), S::Err> {
        if self.memory.has_battery_backed_sram()
            && let Some(sram) = self.memory.sram()
        {
            let checksum = CRC.checksum(sram);
            if checksum != self.last_sram_checksum {
                save_writer.persist_bytes("sav", sram)?;
                self.memory.write_auxiliary_save_files(save_writer)?;

                self.last_sram_checksum = checksum;
            }
        }

        Ok(())
    }

    pub fn copy_cgram(&self, out: &mut [Color]) {
        self.ppu.copy_cgram(out);
    }

    pub fn copy_vram_2bpp(&self, out: &mut [Color], palette: u8, row_len: usize) {
        self.ppu.copy_vram_2bpp(out, palette, row_len);
    }

    pub fn copy_vram_4bpp(&self, out: &mut [Color], palette: u8, row_len: usize) {
        self.ppu.copy_vram_4bpp(out, palette, row_len);
    }

    pub fn copy_vram_8bpp(&self, out: &mut [Color], row_len: usize) {
        self.ppu.copy_vram_8bpp(out, row_len);
    }

    pub fn copy_vram_mode7(&self, out: &mut [Color], row_len: usize) {
        self.ppu.copy_vram_mode7(out, row_len);
    }

    fn render_options(&self) -> RenderFrameOptions {
        let frame_size = self.ppu.frame_size();
        let aspect_ratio = self.aspect_ratio.to_pixel_aspect_ratio(frame_size);

        RenderFrameOptions {
            pixel_aspect_ratio: aspect_ratio,
            composite_params: Some(self.ppu.composite_params()),
            ntsc_per_frame_params: Some(NtscPerFrameParams {
                frame_phase_offset: 2 * self.ppu.frame_start_cycles(),
                per_line_phase_offset: 2 * ppu::MCLKS_PER_NORMAL_SCANLINE,
            }),
            ..RenderFrameOptions::default()
        }
    }

    /// `EmulatorTrait::tick`; with `hold_cpu`, CPU cycles pass without the CPU executing (DMA,
    /// memory refresh, PPU, APU and timers run as usual) and last 6 master cycles, or 8 with
    /// `hold_slow`
    #[inline(always)]
    #[allow(clippy::type_complexity)]
    fn tick_inner<R, A, I, S>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        input_poller: &mut I,
        save_writer: &mut S,
        hold_cpu: bool,
        hold_slow: bool,
    ) -> Result<TickEffect, SnesError<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        I: InputPoller<SnesInputs>,
        S: SaveWriter,
    {
        let (master_cycles_elapsed, pending_write) = if self.memory_refresh_pending {
            // The CPU (including DMA) halts for 40 cycles partway through every scanline so that
            // the system can refresh DRAM (used for work RAM)
            self.memory_refresh_pending = false;
            (MEMORY_REFRESH_CYCLES, None)
        } else {
            let mut bus = new_bus!(self);

            match self.dma_unit.tick(
                &mut bus,
                self.total_master_cycles,
                bus::next_cpu_cycle_mclk(&self.main_cpu),
            ) {
                DmaStatus::None => {
                    // DMA not in progress, tick CPU
                    if hold_cpu {
                        // The cycle passes without the CPU executing anything
                        bus.idle();
                        if hold_slow {
                            bus.access_master_cycles = bus::SLOW_MASTER_CYCLES;
                        }
                    } else {
                        if self.main_cpu.will_fetch_opcode() {
                            self.cpu_counters.opcode_fetches += 1;
                        }
                        self.main_cpu.tick(&mut bus);
                    }
                    self.latched_interrupts = None;

                    bus.cpu_registers.tick_cpu_cycle();

                    self.cpu_counters.busy_master_cycles += bus.access_master_cycles;
                    (bus.access_master_cycles, bus.pending_write)
                }
                DmaStatus::InProgress { master_cycles_elapsed } => {
                    // Latch interrupt lines at the start of DMA to emulate interrupt tests being
                    // delayed by one cycle after the DMA ends.
                    // Wild Guns depends on this
                    if self.latched_interrupts.is_none() {
                        self.latched_interrupts =
                            Some(LatchedInterrupts { nmi: bus.nmi(), irq: bus.irq() });
                    }

                    (master_cycles_elapsed, None)
                }
            }
        };
        debug_assert!(master_cycles_elapsed > 0);

        // Copy WRIO from CPU to PPU for possible H/V counter latching
        self.ppu.update_wrio(self.cpu_registers.wrio_register());

        // Possibly latch H/V counter from the controller (e.g. Super Scope)
        if let Some((h, v)) = self.cpu_registers.controller_hv_latch() {
            self.ppu.update_controller_hv_latch(h, v, master_cycles_elapsed);
        }

        if let ApuTickEffect::OutputSample(sample_l, sample_r) =
            self.apu.tick(master_cycles_elapsed)
        {
            self.audio_resampler.collect_sample(sample_l, sample_r);
            // Samples come out only after a collected one
            self.audio_resampler.output_samples(audio_output).map_err(SnesError::AudioOutput)?;
        }

        self.memory.tick(master_cycles_elapsed);

        let prev_scanline_mclk = self.ppu.scanline_master_cycles();
        let mut tick_effect = TickEffect::None;
        if self.ppu.tick(master_cycles_elapsed) == PpuTickEffect::FrameComplete {
            renderer
                .render_frame(
                    self.ppu.frame_buffer(),
                    self.ppu.frame_size(),
                    self.target_fps(),
                    self.render_options(),
                )
                .map_err(SnesError::Render)?;

            // Only persist SRAM if it's changed since the last write, and only check ~twice per
            // second because of the checksum calculation
            if self.frame_count.is_multiple_of(30) {
                self.persist_sram_if_changed(save_writer).map_err(SnesError::SaveWrite)?;
            }

            self.frame_count += 1;
            tick_effect = TickEffect::FrameRendered;
        }

        let inputs = input_poller.poll();
        self.cpu_registers.tick(master_cycles_elapsed, &self.ppu, inputs);

        // CPU reads are applied before advancing other components but CPU writes are applied after.
        // This fixes freezing in Rendering Ranger R2
        // TODO this is slightly inaccurate - reads should happen 4 mclks before writes, not a full CPU cycle
        if let Some((address, value)) = pending_write {
            self.apply_cpu_write(address, value);
        }

        self.total_master_cycles += master_cycles_elapsed;
        if prev_scanline_mclk < MEMORY_REFRESH_MCLK
            && self.ppu.scanline_master_cycles() >= MEMORY_REFRESH_MCLK
        {
            self.memory_refresh_pending = true;
        }

        // Hook detection only observes the finished tick (DMA, refresh and interrupt polling are
        // untouched). Held ticks leave the CPU where it is, so they cannot arrive anywhere
        if !hold_cpu {
            self.detect_hook_hit();
        }

        Ok(tick_effect)
    }
}

impl EmulatorTrait for SnesEmulator {
    type Button = SnesButton;
    type Inputs = SnesInputs;
    type Config = SnesEmulatorConfig;
    type SaveState = Self;

    type Err<
        RErr: Debug + Display + Send + Sync + 'static,
        AErr: Debug + Display + Send + Sync + 'static,
        SErr: Debug + Display + Send + Sync + 'static,
    > = SnesError<RErr, AErr, SErr>;

    fn tick<R, A, I, S>(
        &mut self,
        renderer: &mut R,
        audio_output: &mut A,
        input_poller: &mut I,
        save_writer: &mut S,
    ) -> Result<TickEffect, Self::Err<R::Err, A::Err, S::Err>>
    where
        R: Renderer,
        A: AudioOutput,
        I: InputPoller<Self::Inputs>,
        S: SaveWriter,
    {
        self.tick_inner(renderer, audio_output, input_poller, save_writer, false, false)
    }

    fn force_render<R>(&mut self, renderer: &mut R) -> Result<(), R::Err>
    where
        R: Renderer,
    {
        let frame_size = self.ppu.frame_size();
        renderer.render_frame(
            self.ppu.frame_buffer(),
            frame_size,
            self.target_fps(),
            self.render_options(),
        )
    }

    fn reload_config(&mut self, config: &Self::Config) {
        self.aspect_ratio = config.aspect_ratio;
        self.ppu.update_config(*config);
        self.apu.update_config(*config);
        self.memory.update_gsu_overclock_factor(config.gsu_overclock_factor);
        self.cpu_registers.reload_config(config);

        self.emulator_config = *config;
    }

    fn soft_reset(&mut self) {
        log::info!("Soft resetting");

        // Reset memory before CPU because some coprocessors (Super FX) block access to the
        // RESET interrupt vector while the coprocessor is running
        self.memory.reset();

        self.main_cpu.reset(&mut new_bus!(self));
        self.cpu_registers.reset();
        self.ppu.reset();
        self.apu.reset();
    }

    fn hard_reset<S: SaveWriter>(&mut self, save_writer: &mut S) {
        log::info!("Hard resetting");

        // A plain mapper's ROM is moved to the new emulator, not copied
        let rom = self.memory.take_rom_image();

        let coprocessor_roms = mem::take(&mut self.coprocessor_roms);
        let line_log = self.ppu.line_log_enabled();
        let compose = self.ppu.compose_pixels();
        let resampling = self.audio_resampler.enabled();
        *self = Self::create_from_image(rom, self.emulator_config, coprocessor_roms, save_writer)
            .expect("Hard resetting should never fail to load");
        self.ppu.set_line_log(line_log);
        self.ppu.set_compose_pixels(compose);
        self.audio_resampler.set_enabled(resampling);
    }

    fn load_state(&mut self, mut state: Self::SaveState) {
        state.memory.take_rom_from(&mut self.memory);
        // Fields that are not encoded (FakeEncode) are carried over from `self`; writes recorded
        // before the load belong to the discarded timeline, so only the log setting is kept
        state.coprocessor_roms = mem::take(&mut self.coprocessor_roms);
        state.cpu_write_log =
            CpuWriteLog { enabled: self.cpu_write_log.enabled, writes: Vec::new() };
        state.cpu_counters = self.cpu_counters;
        state.ppu.set_line_log(self.ppu.line_log_enabled());
        state.ppu.set_compose_pixels(self.ppu.compose_pixels());
        state.audio_resampler.set_enabled(self.audio_resampler.enabled());
        state.dma_unit.carry_over_hdma_copy_units(&self.dma_unit);
        state.cpu_registers.carry_over_nmi_count(&self.cpu_registers);
        state.hooks = Hooks { bitmap: self.hooks.bitmap.take(), hit: None };
        *self = state;
        // A state saved at a hooked opcode fetch arrives there again
        self.detect_hook_hit();
    }

    fn to_save_state(&self) -> Self::SaveState {
        self.partial_clone()
    }

    fn target_fps(&self) -> f64 {
        match (self.timing_mode, self.emulator_config.audio_60hz_hack) {
            (TimingMode::Ntsc, true) => 60.0,
            (TimingMode::Ntsc, false) => 60.0988,
            (TimingMode::Pal, true) => 50.0,
            (TimingMode::Pal, false) => 50.007,
        }
    }

    fn update_audio_output_frequency(&mut self, output_frequency: u64) {
        self.audio_resampler.update_output_frequency(output_frequency);
    }
}
