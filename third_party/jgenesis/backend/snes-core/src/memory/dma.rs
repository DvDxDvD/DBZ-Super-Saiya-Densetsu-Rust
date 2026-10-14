//! SNES general-purpose DMA and HBlank DMA code

use crate::bus::Bus;
use crate::memory::{CpuInternalRegisters, DmaDirection, DmaIncrementMode, HdmaAddressingMode};
use crate::ppu::Ppu;
use bincode::{Decode, Encode};
use jgenesis_common::num::GetBit;
use jgenesis_proc_macros::{FakeDecode, FakeEncode};
use wdc65816_emu::traits::BusInterface;

const CHANNELS: usize = 8;

// Bus B (8-bit) is mapped to $2100-$21FF in Bus A (24-bit)
const BUS_B_BASE_ADDRESS: u32 = 0x002100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum HDmaState {
    Idle,
    ReloadPending,
    Pending,
    Transfer { channel: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum GpDmaState {
    Idle,
    Pending,
    Transfer { channel: u8, bytes_copied: u16 },
}

/// Number of HDMA copy units performed (not part of save states)
#[derive(Debug, Clone, Copy, Default, FakeEncode, FakeDecode)]
struct HdmaCopyUnits(u64);

#[derive(Debug, Clone, Encode, Decode)]
pub struct DmaUnit {
    hdma: HDmaState,
    gpdma: GpDmaState,
    hdma_do_transfer: [bool; CHANNELS],
    last_scanline_mclk: u64,
    dma_active: bool,
    dma_start_mclk: u64,
    // Instrumentation, not encoded: carried over by `SnesEmulator::load_state`
    hdma_copy_units: HdmaCopyUnits,
}

impl DmaUnit {
    pub fn new() -> Self {
        Self {
            hdma: HDmaState::Idle,
            gpdma: GpDmaState::Idle,
            hdma_do_transfer: [false; CHANNELS],
            last_scanline_mclk: 0,
            dma_active: false,
            dma_start_mclk: 0,
            hdma_copy_units: HdmaCopyUnits::default(),
        }
    }

    /// HDMA copy units performed so far: one per active channel on each scanline that transfers
    /// data (line counter reloads and table address loads are not copy units); not part of save
    /// states
    pub fn hdma_copy_units(&self) -> u64 {
        self.hdma_copy_units.0
    }

    /// Takes the HDMA copy-unit counter from `other` (non-encoded fields are carried over when a
    /// state is loaded)
    pub fn carry_over_hdma_copy_units(&mut self, other: &Self) {
        self.hdma_copy_units = other.hdma_copy_units;
    }

    pub fn tick(
        &mut self,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64 + Copy,
    ) -> DmaStatus {
        let last_scanline_mclk = self.last_scanline_mclk;
        self.last_scanline_mclk = bus.ppu.scanline_master_cycles();

        // HDMA takes priority over GPDMA if both are active
        if let Some(status) =
            self.tick_hdma(bus, last_scanline_mclk, total_master_cycles, next_cpu_cycle_mclk)
        {
            return status;
        }

        self.tick_gpdma(bus, total_master_cycles, next_cpu_cycle_mclk)
    }

    /// No HDMA or GPDMA in any state and no DMA cycles in progress
    pub fn is_idle(&self) -> bool {
        self.hdma == HDmaState::Idle && self.gpdma == GpDmaState::Idle && !self.dma_active
    }

    /// The next `tick` takes no DMA cycle (it returns `DmaStatus::None`, so the CPU runs): no DMA
    /// cycles in progress, and HDMA and GPDMA idle or pending with no channel to serve. The
    /// frame-start reload check makes HDMA pending even when no channel is enabled; that state
    /// ends on the next tick without taking a cycle
    pub fn lets_cpu_run(&self, registers: &CpuInternalRegisters) -> bool {
        if self.is_idle() {
            return true;
        }
        let hdma = match self.hdma {
            HDmaState::Idle => true,
            HDmaState::ReloadPending => !registers.active_hdma_channels.into_iter().any(|b| b),
            HDmaState::Pending => !(0..CHANNELS).any(|channel| {
                registers.active_hdma_channels[channel] && registers.hdma_line_counter[channel] != 0
            }),
            HDmaState::Transfer { .. } => false,
        };
        let gpdma = match self.gpdma {
            GpDmaState::Idle => true,
            GpDmaState::Pending => !registers.active_gpdma_channels.into_iter().any(|b| b),
            GpDmaState::Transfer { .. } => false,
        };
        !self.dma_active && hdma && gpdma
    }

    /// HDMA is pending, reloading or transferring
    pub fn hdma_busy(&self) -> bool {
        self.hdma != HDmaState::Idle
    }

    /// Master cycles from the PPU's position until HDMA next takes the bus: 0 if it is busy or
    /// the next DMA tick starts it (the checks of `check_hdma_reload` or `check_hdma_start`
    /// already hold); else the frame-start reload (V=0, H=4) if a channel is enabled in HDMAEN,
    /// or the next per-line transfer (H=276 on a line before vblank) if a channel is active;
    /// `u64::MAX` if no channel is enabled. The checks run on the first DMA tick at or past
    /// these positions, so HDMA never comes sooner
    pub fn master_cycles_until_hdma(&self, ppu: &Ppu, registers: &CpuInternalRegisters) -> u64 {
        // The positions of `check_hdma_reload` and `check_hdma_start`
        const RELOAD_SCANLINE_MCLK: u64 = 4 * 4;
        const START_SCANLINE_MCLK: u64 = 276 * 4;

        if self.hdma != HDmaState::Idle {
            return 0;
        }
        if !registers.active_hdma_channels.into_iter().any(|b| b) {
            // A reload with no channel enabled takes no cycles
            return u64::MAX;
        }
        let line = ppu.scanline();
        let mclk = ppu.scanline_master_cycles();
        let last = self.last_scanline_mclk;
        let display = |line: u16| line < ppu.first_vblank_scanline();
        let active = (0..CHANNELS).any(|channel| {
            registers.active_hdma_channels[channel] && registers.hdma_line_counter[channel] != 0
        });
        let reload_due = line == 0
            && mclk >= RELOAD_SCANLINE_MCLK
            && (last < RELOAD_SCANLINE_MCLK || last > mclk);
        let start_due =
            active && display(line) && mclk >= START_SCANLINE_MCLK && last < START_SCANLINE_MCLK;
        if reload_due || start_due {
            return 0;
        }

        let reload = ppu.master_cycles_until(0, RELOAD_SCANLINE_MCLK);
        if !active {
            // Until the reload no line transfers
            return reload;
        }
        let start = [line, line + 1, 0]
            .into_iter()
            .filter(|&line| display(line))
            .map(|line| ppu.master_cycles_until(line, START_SCANLINE_MCLK))
            .min()
            .unwrap_or(u64::MAX);
        reload.min(start)
    }

    fn tick_hdma(
        &mut self,
        bus: &mut Bus<'_>,
        last_scanline_mclk: u64,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> Option<DmaStatus> {
        match self.hdma {
            HDmaState::Idle => {
                if Self::check_hdma_reload(bus, last_scanline_mclk) {
                    if self.dma_active {
                        return self.hdma_reload(bus, total_master_cycles, next_cpu_cycle_mclk);
                    }
                    self.hdma = HDmaState::ReloadPending;
                } else if Self::check_hdma_start(bus, last_scanline_mclk) {
                    if self.dma_active {
                        return self.hdma_start(bus, total_master_cycles);
                    }
                    self.hdma = HDmaState::Pending;
                }

                None
            }
            HDmaState::ReloadPending => {
                self.hdma_reload(bus, total_master_cycles, next_cpu_cycle_mclk)
            }
            HDmaState::Pending => self.hdma_start(bus, total_master_cycles),
            HDmaState::Transfer { channel } => Some(self.hdma_progress_transfer(
                channel.into(),
                bus,
                total_master_cycles,
                next_cpu_cycle_mclk,
            )),
        }
    }

    fn check_hdma_reload(bus: &mut Bus<'_>, last_scanline_mclk: u64) -> bool {
        // HDMA reload begins at H=4 V=0
        const RELOAD_SCANLINE_MCLK: u64 = 4 * 4;

        let scanline_mclk = bus.ppu.scanline_master_cycles();
        bus.ppu.scanline() == 0
            && scanline_mclk >= RELOAD_SCANLINE_MCLK
            && (last_scanline_mclk < RELOAD_SCANLINE_MCLK || last_scanline_mclk > scanline_mclk)
    }

    fn check_hdma_start(bus: &mut Bus<'_>, last_scanline_mclk: u64) -> bool {
        // HDMA begins at H=276, V in 0-224
        const START_SCANLINE_MCLK: u64 = 276 * 4;

        // The cheap tests first: they hold once a line
        !bus.ppu.vblank_flag()
            && bus.ppu.scanline_master_cycles() >= START_SCANLINE_MCLK
            && last_scanline_mclk < START_SCANLINE_MCLK
            && Self::any_hdma_active(bus)
    }

    fn hdma_channel_active(bus: &Bus<'_>, channel: usize) -> bool {
        // HDMA channels go inactive for the rest of the frame when line counter is loaded with 0
        bus.cpu_registers.active_hdma_channels[channel]
            && bus.cpu_registers.hdma_line_counter[channel] != 0
    }

    fn any_hdma_active(bus: &mut Bus<'_>) -> bool {
        (0..CHANNELS).any(|channel| Self::hdma_channel_active(bus, channel))
    }

    fn hdma_reload(
        &mut self,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> Option<DmaStatus> {
        if !bus.cpu_registers.active_hdma_channels.into_iter().any(|b| b) {
            // Reset all do_transfer flags at reload time when no HDMA channels are active
            // Super Ghouls 'N Ghosts depends on this because it enables HDMA mid-frame
            self.hdma_do_transfer.fill(false);
            self.hdma = HDmaState::Idle;
            return None;
        }

        // 8-cycle overhead for HDMA reload if any channels are active
        let mut cycles = 8 + self.start_dma_if_inactive(total_master_cycles);

        for channel in 0..CHANNELS {
            if !bus.cpu_registers.active_hdma_channels[channel] {
                self.hdma_do_transfer[channel] = false;
                continue;
            }

            log::trace!("Reloading HDMA channel {channel}");

            // Active HDMA cancels any active GPDMA on the same channel
            bus.cpu_registers.active_gpdma_channels[channel] = false;

            bus.cpu_registers.hdma_table_current_address[channel] =
                bus.cpu_registers.gpdma_current_address[channel];

            // 8 cycles for each active channel, +16 if in indirect mode
            cycles += 8 + self.hdma_reload_line_counter(bus, channel);
        }

        self.hdma = HDmaState::Idle;
        cycles += self.end_dma_if_done(bus, total_master_cycles + cycles, next_cpu_cycle_mclk);

        Some(DmaStatus::InProgress { master_cycles_elapsed: cycles })
    }

    #[must_use]
    fn hdma_reload_line_counter(&mut self, bus: &mut Bus<'_>, channel: usize) -> u64 {
        log::trace!("Reloading HDMA line counter for channel {channel}");

        let bank = bus.cpu_registers.dma_bank[channel];
        let mut current_addr = bus.cpu_registers.hdma_table_current_address[channel];

        log::trace!("  HDMA table bank={bank:02X}, current address={current_addr:04X}");

        let line_counter = bus.read(u24(bank, current_addr));
        bus.cpu_registers.hdma_line_counter[channel] = line_counter;
        current_addr = current_addr.wrapping_add(1);

        log::trace!("  HDMA line counter: {line_counter:02X}");

        let mut cycles = 0;
        if bus.cpu_registers.hdma_addressing_mode[channel] == HdmaAddressingMode::Indirect {
            cycles = 16;

            let address_lsb = bus.read(u24(bank, current_addr));
            current_addr = current_addr.wrapping_add(1);
            let address_msb = bus.read(u24(bank, current_addr));
            current_addr = current_addr.wrapping_add(1);

            // Same register is used for GPDMA byte counter and HDMA indirect address
            let address = u16::from_le_bytes([address_lsb, address_msb]);
            bus.cpu_registers.gpdma_byte_counter[channel] = address;

            log::trace!(
                "  HDMA indirect bank = {:02X}, indirect address: {address:04X}",
                bus.cpu_registers.hdma_indirect_bank[channel]
            );
        }

        bus.cpu_registers.hdma_table_current_address[channel] = current_addr;
        self.hdma_do_transfer[channel] = true;

        cycles
    }

    fn hdma_start(&mut self, bus: &mut Bus<'_>, total_master_cycles: u64) -> Option<DmaStatus> {
        if !Self::any_hdma_active(bus) {
            self.hdma = HDmaState::Idle;
            return None;
        }

        // 8-cycle overhead at HDMA start if any channels are active
        let cycles = 8 + self.start_dma_if_inactive(total_master_cycles);
        self.hdma = HDmaState::Transfer { channel: 0 };

        Some(DmaStatus::InProgress { master_cycles_elapsed: cycles })
    }

    fn hdma_progress_transfer(
        &mut self,
        mut channel: usize,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> DmaStatus {
        while channel < CHANNELS {
            if !Self::hdma_channel_active(bus, channel) {
                channel += 1;
                continue;
            }

            // Active HDMA cancels any active GPDMA on the same channel
            bus.cpu_registers.active_gpdma_channels[channel] = false;

            if !self.hdma_do_transfer[channel] {
                channel += 1;
                continue;
            }

            // 8 cycles per byte copied (up to 4 bytes / 32 cycles)
            let cycles = hdma_copy_unit(bus, channel);
            self.hdma_copy_units.0 += 1;
            self.hdma = HDmaState::Transfer { channel: (channel + 1) as u8 };
            return DmaStatus::InProgress { master_cycles_elapsed: cycles };
        }

        self.hdma_finish(bus, total_master_cycles, next_cpu_cycle_mclk)
    }

    fn hdma_finish(
        &mut self,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> DmaStatus {
        let mut cycles = 0;

        for channel in 0..CHANNELS {
            if !Self::hdma_channel_active(bus, channel) {
                continue;
            }

            let line_counter = bus.cpu_registers.hdma_line_counter[channel].wrapping_sub(1);
            bus.cpu_registers.hdma_line_counter[channel] = line_counter;

            // Highest bit of line counter functions as a repeat flag
            self.hdma_do_transfer[channel] = line_counter.bit(7);

            // 8 cycles per active channel
            // +16 for each channel in indirect mode that reloads its line counter
            cycles += 8;

            // Line counter reloads when lowest 7 bits are 0
            if line_counter & 0x7F == 0 {
                cycles += self.hdma_reload_line_counter(bus, channel);
            }
        }

        self.hdma = HDmaState::Idle;
        cycles += self.end_dma_if_done(bus, total_master_cycles + cycles, next_cpu_cycle_mclk);

        debug_assert_ne!(cycles, 0);
        DmaStatus::InProgress { master_cycles_elapsed: cycles }
    }

    fn tick_gpdma(
        &mut self,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> DmaStatus {
        match self.gpdma {
            GpDmaState::Idle => {
                if bus.cpu_registers.gpdma_requested() {
                    self.gpdma = GpDmaState::Pending;
                }
                DmaStatus::None
            }
            GpDmaState::Pending => self.gpdma_start(bus, total_master_cycles),
            GpDmaState::Transfer { channel, bytes_copied } => self.gpdma_progress_transfer(
                channel.into(),
                bytes_copied,
                bus,
                total_master_cycles,
                next_cpu_cycle_mclk,
            ),
        }
    }

    fn gpdma_start(&mut self, bus: &mut Bus<'_>, total_master_cycles: u64) -> DmaStatus {
        if !bus.cpu_registers.active_gpdma_channels.into_iter().any(|b| b) {
            self.gpdma = GpDmaState::Idle;
            return DmaStatus::None;
        }

        if log::log_enabled!(log::Level::Trace) {
            gpdma_start_log(bus);
        }

        // 8-cycle overhead when starting GPDMA
        let cycles = 8 + self.start_dma_if_inactive(total_master_cycles);
        self.gpdma = GpDmaState::Transfer { channel: 0, bytes_copied: 0 };
        DmaStatus::InProgress { master_cycles_elapsed: cycles }
    }

    fn gpdma_progress_transfer(
        &mut self,
        mut channel: usize,
        mut bytes_copied: u16,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> DmaStatus {
        while channel < CHANNELS {
            if !bus.cpu_registers.active_gpdma_channels[channel] {
                channel += 1;
                bytes_copied = 0;
                continue;
            }

            // 8 cycles per byte
            let mut cycles = 8;

            if bytes_copied == 0 {
                // 8-cycle overhead per active channel
                cycles += 8;

                // Notify coprocessor when GPDMA starts a new channel; needed by S-DD1 and SA-1
                let start_address = u24(
                    bus.cpu_registers.dma_bank[channel],
                    bus.cpu_registers.gpdma_current_address[channel],
                );
                bus.memory.notify_dma_start(channel as u8, start_address);
            }

            gpdma_copy_byte(bus, channel, bytes_copied);

            if bus.cpu_registers.gpdma_byte_counter[channel] == 0 {
                bus.cpu_registers.active_gpdma_channels[channel] = false;
                channel += 1;
                bytes_copied = 0;
            } else {
                bytes_copied = bytes_copied.wrapping_add(1);
            }

            self.gpdma = GpDmaState::Transfer { channel: channel as u8, bytes_copied };
            return DmaStatus::InProgress { master_cycles_elapsed: cycles };
        }

        bus.memory.notify_dma_end();

        self.gpdma = GpDmaState::Idle;
        let cycles = self.end_dma_if_done(bus, total_master_cycles, next_cpu_cycle_mclk);
        DmaStatus::InProgress { master_cycles_elapsed: cycles }
    }

    #[must_use]
    fn start_dma_if_inactive(&mut self, total_master_cycles: u64) -> u64 {
        if self.dma_active {
            return 0;
        }

        self.dma_active = true;
        self.dma_start_mclk = total_master_cycles;

        // DMA can only begin at a multiple of 8 cycles since power-on
        8 - (total_master_cycles % 8)
    }

    #[must_use]
    fn end_dma_if_done(
        &mut self,
        bus: &mut Bus<'_>,
        total_master_cycles: u64,
        next_cpu_cycle_mclk: impl Fn(&Bus<'_>) -> u64,
    ) -> u64 {
        if self.hdma != HDmaState::Idle || self.gpdma != GpDmaState::Idle {
            return 0;
        }

        self.dma_active = false;

        // After DMA ends, must wait until a whole number of CPU cycles have elapsed since DMA began
        // CPU clock timing is based on the cycle that will execute after DMA ends
        let dma_elapsed_mclk = total_master_cycles - self.dma_start_mclk;
        let next_cpu_cycle_mclk = next_cpu_cycle_mclk(bus);
        next_cpu_cycle_mclk - (dma_elapsed_mclk % next_cpu_cycle_mclk)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaStatus {
    None,
    InProgress { master_cycles_elapsed: u64 },
}

fn u24(bank: u8, offset: u16) -> u32 {
    (u32::from(bank) << 16) | u32::from(offset)
}

// Returns number of cycles (8 * bytes copied)
fn hdma_copy_unit(bus: &mut Bus<'_>, channel: usize) -> u64 {
    let (bus_a_bank, mut bus_a_offset) = match bus.cpu_registers.hdma_addressing_mode[channel] {
        HdmaAddressingMode::Direct => (
            bus.cpu_registers.dma_bank[channel],
            bus.cpu_registers.hdma_table_current_address[channel],
        ),
        HdmaAddressingMode::Indirect => (
            bus.cpu_registers.hdma_indirect_bank[channel],
            bus.cpu_registers.gpdma_byte_counter[channel],
        ),
    };

    let bus_b_address = bus.cpu_registers.dma_bus_b_address[channel];
    let direction = bus.cpu_registers.dma_direction[channel];

    log::trace!(
        "HDMA channel {channel}: ABank={bus_a_bank:02X}, AAddr={bus_a_offset:04X}, BAddr={bus_b_address:02X}, Unit={}, Direction={direction:?}, AddrMode={:?}",
        bus.cpu_registers.dma_transfer_unit[channel],
        bus.cpu_registers.hdma_addressing_mode[channel]
    );

    let bytes_copied = match bus.cpu_registers.dma_transfer_unit[channel] {
        0 => {
            // 1 byte, 1 register
            hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);

            1
        }
        1 => {
            // 2 bytes, 2 registers
            hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);
            hdma_copy_byte(
                direction,
                bus,
                bus_a_bank,
                &mut bus_a_offset,
                bus_b_address.wrapping_add(1),
            );

            2
        }
        2 | 6 => {
            // 2 bytes, 1 register
            hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);
            hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);

            2
        }
        3 | 7 => {
            // 4 bytes, 2 registers (serial)
            hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);
            hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);
            hdma_copy_byte(
                direction,
                bus,
                bus_a_bank,
                &mut bus_a_offset,
                bus_b_address.wrapping_add(1),
            );
            hdma_copy_byte(
                direction,
                bus,
                bus_a_bank,
                &mut bus_a_offset,
                bus_b_address.wrapping_add(1),
            );

            4
        }
        4 => {
            // 4 bytes, 4 registers
            for i in 0..4 {
                hdma_copy_byte(
                    direction,
                    bus,
                    bus_a_bank,
                    &mut bus_a_offset,
                    bus_b_address.wrapping_add(i),
                );
            }

            4
        }
        5 => {
            // 4 bytes, 2 registers (alternating)
            for _ in 0..2 {
                hdma_copy_byte(direction, bus, bus_a_bank, &mut bus_a_offset, bus_b_address);
                hdma_copy_byte(
                    direction,
                    bus,
                    bus_a_bank,
                    &mut bus_a_offset,
                    bus_b_address.wrapping_add(1),
                );
            }

            4
        }
        _ => panic!("invalid DMA transfer unit: {}", bus.cpu_registers.dma_transfer_unit[channel]),
    };

    log::trace!("  Copied {bytes_copied} bytes, new AAddress: {bus_a_offset:04X}");

    // Write back incremented bus A address
    match bus.cpu_registers.hdma_addressing_mode[channel] {
        HdmaAddressingMode::Direct => {
            bus.cpu_registers.hdma_table_current_address[channel] = bus_a_offset;
        }
        HdmaAddressingMode::Indirect => {
            bus.cpu_registers.gpdma_byte_counter[channel] = bus_a_offset;
        }
    }

    // Each byte copy takes 8 cycles
    8 * bytes_copied
}

fn hdma_copy_byte(
    direction: DmaDirection,
    bus: &mut Bus<'_>,
    bus_a_bank: u8,
    bus_a_offset: &mut u16,
    bus_b_address: u8,
) {
    let bus_a_full_address = u24(bus_a_bank, *bus_a_offset);
    *bus_a_offset = bus_a_offset.wrapping_add(1);

    let bus_b_full_address = BUS_B_BASE_ADDRESS | u32::from(bus_b_address);

    match direction {
        DmaDirection::AtoB => {
            let byte = dma_read_bus_a(bus, bus_a_full_address);
            bus.apply_write(bus_b_full_address, byte);
        }
        DmaDirection::BtoA => {
            let byte = bus.read(bus_b_full_address);
            dma_write_bus_a(bus, bus_a_full_address, byte);
        }
    }
}

fn dma_read_bus_a(bus: &mut Bus<'_>, bus_a_address: u32) -> u8 {
    let bank = (bus_a_address >> 16) & 0xFF;
    let offset = bus_a_address & 0xFFFF;
    match (bank, offset) {
        // DMA cannot read bus B or DMA registers through bus A
        // Krusty's Super Fun House depends on this or else it will write incorrect BG color
        // palettes to CGRAM
        (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x21FF | 0x4300..=0x43FF) => bus.memory.cpu_open_bus(),
        _ => bus.read(bus_a_address),
    }
}

fn dma_write_bus_a(bus: &mut Bus<'_>, bus_a_address: u32, value: u8) {
    let bank = (bus_a_address >> 16) & 0xFF;
    let offset = bus_a_address & 0xFFFF;
    match (bank, offset) {
        // DMA cannot write to bus B or DMA registers through bus A
        (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x21FF | 0x4300..=0x43FF) => {}
        _ => bus.apply_write(bus_a_address, value),
    }
}

fn gpdma_copy_byte(bus: &mut Bus<'_>, channel: usize, bytes_copied: u16) {
    let bus_a_bank = bus.cpu_registers.dma_bank[channel];
    let bus_a_address = bus.cpu_registers.gpdma_current_address[channel];
    let bus_a_full_address = (u32::from(bus_a_bank) << 16) | u32::from(bus_a_address);

    // Transfer units (0-7):
    //   0: 1 byte, 1 register
    //   1: 2 bytes, 2 registers
    //   2: 2 bytes, 1 register (functionally same as 0 for GPDMA)
    //   3: 4 bytes, 2 registers (xx, xx, xx+1, xx+1)
    //   4: 4 bytes, 4 registers
    //   5: 4 bytes, 2 registers alternating (xx, xx+1, xx, xx+1) (functionally same as 1 for GPDMA)
    //   6: Same as 2
    //   7: Same as 3
    let transfer_unit = bus.cpu_registers.dma_transfer_unit[channel];
    let bus_b_adjustment = match transfer_unit {
        0 | 2 | 6 => 0,
        1 | 5 => (bytes_copied & 0x01) as u8,
        3 | 7 => ((bytes_copied >> 1) & 0x01) as u8,
        4 => (bytes_copied & 0x03) as u8,
        _ => panic!("invalid transfer unit: {transfer_unit}"),
    };

    let bus_b_address = BUS_B_BASE_ADDRESS
        | u32::from(bus.cpu_registers.dma_bus_b_address[channel].wrapping_add(bus_b_adjustment));

    match bus.cpu_registers.dma_direction[channel] {
        DmaDirection::AtoB => {
            let byte = dma_read_bus_a(bus, bus_a_full_address);
            bus.apply_write(bus_b_address, byte);
        }
        DmaDirection::BtoA => {
            let byte = bus.read(bus_b_address);
            dma_write_bus_a(bus, bus_a_full_address, byte);
        }
    }

    match bus.cpu_registers.dma_increment_mode[channel] {
        DmaIncrementMode::Fixed0 | DmaIncrementMode::Fixed1 => {}
        DmaIncrementMode::Increment => {
            bus.cpu_registers.gpdma_current_address[channel] = bus_a_address.wrapping_add(1);
        }
        DmaIncrementMode::Decrement => {
            bus.cpu_registers.gpdma_current_address[channel] = bus_a_address.wrapping_sub(1);
        }
    }

    bus.cpu_registers.gpdma_byte_counter[channel] =
        bus.cpu_registers.gpdma_byte_counter[channel].wrapping_sub(1);
}

fn gpdma_start_log(bus: &Bus<'_>) {
    log::trace!("GPDMA started");
    for (i, active) in bus.cpu_registers.active_gpdma_channels.iter().copied().enumerate() {
        if !active {
            continue;
        }

        log::trace!("  Channel {i} bus A bank: {:02X}", bus.cpu_registers.dma_bank[i]);
        log::trace!(
            "  Channel {i} bus A address: {:04X}",
            bus.cpu_registers.gpdma_current_address[i]
        );
        log::trace!("  Channel {i} bus B address: {:02X}", bus.cpu_registers.dma_bus_b_address[i]);
        log::trace!("  Channel {i} byte counter: {:04X}", bus.cpu_registers.gpdma_byte_counter[i]);
        log::trace!("  Channel {i} direction: {:?}", bus.cpu_registers.dma_direction[i]);
        log::trace!("  Channel {i} transfer unit: {}", bus.cpu_registers.dma_transfer_unit[i]);
        log::trace!("  Channel {i} increment mode: {:?}", bus.cpu_registers.dma_increment_mode[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CoprocessorRoms, SnesEmulatorConfig};
    use crate::apu::Apu;
    use crate::memory::Memory;
    use jgenesis_common::frontend::{SaveWriter, TimingMode};
    use std::io;
    use std::num::NonZeroU64;

    /// No save files
    struct NoSaves;

    impl SaveWriter for NoSaves {
        type Err = io::Error;

        fn load_bytes(&mut self, _extension: &str) -> Result<Vec<u8>, Self::Err> {
            Err(io::ErrorKind::NotFound.into())
        }

        fn persist_bytes(&mut self, _extension: &str, _bytes: &[u8]) -> Result<(), Self::Err> {
            Ok(())
        }

        fn load_serialized<D: Decode<()>>(&mut self, _extension: &str) -> Result<D, Self::Err> {
            Err(io::ErrorKind::NotFound.into())
        }

        fn persist_serialized<E: Encode>(
            &mut self,
            _extension: &str,
            _data: E,
        ) -> Result<(), Self::Err> {
            Ok(())
        }
    }

    /// What a `Bus` borrows, around a blank 32 KiB `LoROM`
    struct Parts {
        memory: Memory,
        registers: CpuInternalRegisters,
        ppu: Ppu,
        apu: Apu,
    }

    impl Parts {
        fn new() -> Self {
            let config = SnesEmulatorConfig::default();
            let mut rom = vec![0; 0x8000];
            rom[0x7FD5] = 0x20; // LoROM
            // HDMA tables at $00:8100 (254 lines, a byte each, then the end) and $00:8120 (2 lines,
            // then the end)
            rom[0x100..0x105].copy_from_slice(&[0xFF, 0x11, 0xFF, 0x22, 0x00]);
            rom[0x120..0x123].copy_from_slice(&[0x82, 0x33, 0x00]);
            let memory = Memory::create(
                rom,
                None,
                &CoprocessorRoms::none(),
                Some(TimingMode::Ntsc),
                NonZeroU64::new(1).unwrap(),
                Some(1),
                &mut NoSaves,
            )
            .unwrap();
            Self {
                memory,
                registers: CpuInternalRegisters::new(&config),
                ppu: Ppu::new(TimingMode::Ntsc, config),
                apu: Apu::new(TimingMode::Ntsc, config),
            }
        }

        /// One DMA unit tick
        fn tick(&mut self, dma: &mut DmaUnit) -> DmaStatus {
            let mut bus = Bus {
                memory: &mut self.memory,
                cpu_registers: &mut self.registers,
                ppu: &mut self.ppu,
                apu: &mut self.apu,
                latched_interrupts: None,
                access_master_cycles: 0,
                pending_write: None,
            };
            dma.tick(&mut bus, 0, |_: &Bus<'_>| 6)
        }
    }

    /// `lets_cpu_run` says whether the next tick takes the bus, and it does so exactly then
    fn assert_next_tick(dma: &mut DmaUnit, parts: &mut Parts, lets_cpu_run: bool) {
        assert_eq!(dma.lets_cpu_run(&parts.registers), lets_cpu_run);
        assert_eq!(parts.tick(dma) == DmaStatus::None, lets_cpu_run);
    }

    #[test]
    fn hdma_start_pending_without_an_active_channel_lets_the_cpu_run() {
        // `check_hdma_start` saw an active channel; the CPU then disabled HDMA in the same tick
        let mut parts = Parts::new();
        let mut dma = DmaUnit::new();
        dma.hdma = HDmaState::Pending;
        assert_next_tick(&mut dma, &mut parts, true);
        assert!(dma.is_idle());

        // A channel whose line counter is 0 is not active
        parts.registers.active_hdma_channels[2] = true;
        parts.registers.hdma_line_counter[2] = 0;
        dma.hdma = HDmaState::Pending;
        assert_next_tick(&mut dma, &mut parts, true);

        parts.registers.hdma_line_counter[2] = 1;
        dma.hdma = HDmaState::Pending;
        assert_next_tick(&mut dma, &mut parts, false);
    }

    #[test]
    fn hdma_reload_pending_without_an_enabled_channel_lets_the_cpu_run() {
        let mut parts = Parts::new();
        let mut dma = DmaUnit::new();
        dma.hdma = HDmaState::ReloadPending;
        assert_next_tick(&mut dma, &mut parts, true);
        assert!(dma.is_idle());

        parts.registers.active_hdma_channels[5] = true;
        dma.hdma = HDmaState::ReloadPending;
        assert_next_tick(&mut dma, &mut parts, false);
    }

    #[test]
    fn gpdma_pending_without_an_enabled_channel_lets_the_cpu_run() {
        let mut parts = Parts::new();
        let mut dma = DmaUnit::new();
        dma.gpdma = GpDmaState::Pending;
        assert_next_tick(&mut dma, &mut parts, true);
        assert!(dma.is_idle());

        parts.registers.active_gpdma_channels[0] = true;
        dma.gpdma = GpDmaState::Pending;
        assert_next_tick(&mut dma, &mut parts, false);
    }

    /// One step of the model: the PPU moves on by 4 master cycles and the DMA unit ticks once;
    /// if it took the bus, it then runs on to idle with the PPU standing still. Returns whether
    /// HDMA took the bus on that first tick.
    fn step(parts: &mut Parts, dma: &mut DmaUnit) -> bool {
        let _ = parts.ppu.tick(4);
        parts.tick(dma);
        let took_the_bus = dma.hdma_busy();
        for _ in 0..1000 {
            if dma.is_idle() {
                return took_the_bus;
            }
            parts.tick(dma);
        }
        panic!("the DMA unit does not go idle");
    }

    #[test]
    fn time_to_hdma() {
        const SETINI: u32 = 0x2133;
        // (timing, SETINI, the table: 254 lines or 2, whether the channel stays active)
        let configs = [
            (TimingMode::Ntsc, 0x00, 0x00, true),
            (TimingMode::Ntsc, 0x04, 0x00, true), // overscan: 239 lines
            (TimingMode::Ntsc, 0x01, 0x00, true), // interlaced
            (TimingMode::Pal, 0x00, 0x00, true),
            (TimingMode::Pal, 0x05, 0x00, true),
            (TimingMode::Ntsc, 0x00, 0x20, false), // the table ends after 2 lines
        ];
        // Master cycles to move on after each event, so the predictions start from many
        // positions (each a multiple of 4)
        const SWEEP: [u64; 8] = [0, 4, 1364 * 3 + 500, 1364 * 100, 96, 1364 * 7, 4096, 1300];

        for (timing_mode, setini, table, long) in configs {
            let mut parts = Parts::new();
            parts.ppu = Ppu::new(timing_mode, SnesEmulatorConfig::default());
            parts.ppu.write_port(SETINI, setini);
            let mut dma = DmaUnit::new();
            let registers = &mut parts.registers;
            assert_eq!(
                dma.master_cycles_until_hdma(&parts.ppu, registers),
                u64::MAX,
                "no channel is enabled"
            );
            // Channel 0: direct, one byte a line to COLDATA, from a table in the ROM
            for (address, value) in [
                (0x4300, 0x00),
                (0x4301, 0x32),
                (0x4302, table),
                (0x4303, 0x81),
                (0x4304, 0x00),
                (0x420C, 0x01),
            ] {
                registers.write_register(address, value);
            }

            for round in 0..700 {
                let what =
                    format!("{timing_mode:?} SETINI {setini:02X} table {table:02X} round {round}");
                let predicted = dma.master_cycles_until_hdma(&parts.ppu, &parts.registers);
                assert!(
                    predicted != u64::MAX && predicted.is_multiple_of(4),
                    "{what}: {predicted}"
                );
                if predicted == 0 {
                    // Due: the next DMA tick starts it
                    assert!(step_without_ppu(&mut parts, &mut dma), "{what}: due");
                } else {
                    for elapsed in (4..=predicted).step_by(4) {
                        let took_the_bus = step(&mut parts, &mut dma);
                        assert_eq!(
                            took_the_bus,
                            elapsed == predicted,
                            "{what}: {elapsed} of {predicted}"
                        );
                    }
                }
                for _ in 0..SWEEP[round % SWEEP.len()] / 4 {
                    step(&mut parts, &mut dma);
                }
            }
            let units = dma.hdma_copy_units();
            assert!(if long { units > 1000 } else { units > 3 }, "{timing_mode:?}: {units}");
        }
    }

    /// The DMA unit ticks once with the PPU standing still, then runs on to idle; returns
    /// whether HDMA took the bus on that tick.
    fn step_without_ppu(parts: &mut Parts, dma: &mut DmaUnit) -> bool {
        parts.tick(dma);
        let took_the_bus = dma.hdma_busy();
        for _ in 0..1000 {
            if dma.is_idle() {
                return took_the_bus;
            }
            parts.tick(dma);
        }
        panic!("the DMA unit does not go idle");
    }
}
