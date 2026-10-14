# Vendored jgenesis

- Upstream: https://github.com/jsgroth/jgenesis
- Commit: 85ffe5fc66fcc196fb3f5dd5340939e6e4f442a2 (2026-10-06)
- License: GPL-3.0 (see LICENSE). This project is released under the GPL-3.0 as well (see the root LICENSE and NOTICE.md).
- Crates: common/jgenesis-common, common/jgenesis-proc-macros, common/dsp, cpu/wdc65816-emu,
  cpu/spc700-emu, cpu/arm7tdmi-emu, config/snes-config, backend/snes-core,
  backend/snes-coprocessors, frontend/jgenesis-renderer

## Local patches

Every patch is a separate commit and gets an entry here: commit, files, reason.

Save-state rule: any patch changing the bincode layout of `SnesEmulator` bumps `FORMAT_VERSION` in `crates/ssd-oracle/src/state.rs` in the same series.

### `patch(jgenesis): optional deterministic WRAM fill`

- Files: `backend/snes-core/src/api.rs`, `backend/snes-core/src/memory.rs`.
- Change: `SnesEmulatorConfig` gains `wram_fill_seed: Option<u64>` (default `None`), passed to
  `Memory::create`. `Some(seed)` fills the 128 KiB of WRAM from a xorshift64* stream (non-zero
  garbage, same bytes every power-on); `None` keeps upstream's `rand::random` fill.
- Reason: power-on WRAM is random upstream, so two fresh machines diverge. Verification, replay and
  the pinned baseline need a bit-exact reproducible start.
- Save-state layout: `SnesEmulatorConfig` is an encoded field of `SnesEmulator`, so the state layout
  changes; `FORMAT_VERSION` in `crates/ssd-oracle/src/state.rs` is bumped to 3 in the same series.

### `patch(jgenesis): runnable snapshots, carried-over fields, SRAM flush`

- Files: `backend/snes-core/src/api.rs`, `backend/snes-core/src/memory/cartridge.rs`,
  `backend/snes-core/src/ppu.rs`.
- Change:
  - `Rom` wraps `Arc<[u8]>` (was `Box<[u8]>`) and derefs to `[u8]`; `take_rom` copies the bytes
    out (`to_vec`), `take_rom_from` rebuilds the `Arc`. The plain `LoRom`/`HiRom` variants drop
    `#[partial_clone(default)]` on `rom`, so `partial_clone` shares the ROM instead of leaving it
    empty. Other mappers are unchanged (their ROM is still skipped).
  - `Ppu::clone_without_frame_buffer` and `SnesEmulator::snapshot()`: a runnable copy that shares
    the ROM and starts with a zeroed frame buffer (`to_save_state()` still copies it). Both build
    the copy with a full struct literal (no `..` base), so a field added later must be handled
    explicitly. Coprocessor ROM loaders are not carried into a snapshot.
  - The SRAM persist block of `tick` moves into `persist_sram_if_changed` (same checks, same
    every-30-frames gate in `tick`); `SnesEmulator::flush_sram` calls it on demand.
  - `SnesEmulator::peek_sram()` exposes the cartridge SRAM read-only.
  - Carry-over rule: fields that are not encoded (`FakeEncode`) are moved from `self` in
    `EmulatorTrait::load_state`, like `coprocessor_roms`; later patches that add such fields extend
    both `load_state` and `snapshot()`.
- Reason: the verifier runs Rust reimplementations on a copy of the live machine, which needs a
  cheap runnable clone (upstream's `to_save_state()` copy has no ROM and copies the frame
  buffer). The frontend must flush SRAM on exit, or up to 30 frames of in-game saves are lost.
- Save-state layout: unchanged (`Rom` is `FakeEncode`; no encoded field added). With no snapshot
  taken and no flush called, emulation is identical to upstream.

### `patch(jgenesis): CPU, bus and memory access for the oracle`

- Files: `cpu/wdc65816-emu/src/core.rs`, `backend/snes-core/Cargo.toml` (`bytemuck`),
  `backend/snes-core/src/api.rs`, `backend/snes-core/src/memory.rs`,
  `backend/snes-core/src/memory/dma.rs`, `backend/snes-core/src/ppu.rs`.
- Change:
  - `Wdc65816::will_fetch_opcode` (the next `tick` takes the opcode-fetch path of `execute`: cycle
    0, not stopped or waiting, no NMI/IRQ/reset to handle) and `Wdc65816::is_handling_interrupt`.
  - `SnesEmulator` accessors: `cpu_registers`/`set_cpu_registers`, `bus_read` (`BusInterface::read`
    with its side effects), `bus_write` (`Bus::apply_write`), `peek_wram`/`peek_vram`/`peek_cgram`/
    `peek_oam` (OAM low table viewed as bytes with `bytemuck`), `total_master_cycles`,
    `hdma_busy`, `gpdma_requested`, `at_fetch_boundary` (no refresh pending, CPU about to fetch,
    `DmaUnit::is_idle`). Helpers: `Memory::wram`, `CpuInternalRegisters::gpdma_requested`,
    `DmaUnit::{is_idle, hdma_busy}`, `Ppu::{vram, cgram, oam}`.
  - CPU-write log: `set_cpu_write_log`/`take_cpu_writes`. Writes are recorded where the CPU's
    `pending_write` is applied in `tick` and in `bus_write`, never in `Bus::apply_write`, which
    HDMA and GPDMA also use.
  - Counters: `cpu_busy_cycles` (sum of the master cycles of the `DmaStatus::None` arm, i.e. CPU
    cycles including held ones) and `opcode_fetches` (ticks that start an instruction, checked
    with `will_fetch_opcode` before the CPU ticks).
  - `tick` becomes `tick_inner(hold_cpu)` (`#[inline(always)]`); with `hold_cpu` the CPU's
    `tick` is replaced by `bus.idle()` (6 master cycles; latched interrupts are still cleared and
    `tick_cpu_cycle` still runs). `advance_idle` ticks held until at least `master_cycles` have
    elapsed and returns whether a frame was rendered.
  - The log and the counters are not encoded (`FakeEncode` newtypes, `#[partial_clone(default)]`);
    `load_state` carries them over from `self`, `snapshot()` keeps the log setting and the
    counters and starts with an empty log. `hard_reset` resets them.
- Reason: the oracle verifies Rust reimplementations against the original routines: it reads and
  writes CPU state and memory, compares the writes a routine makes, detects instruction
  boundaries for hooks and charges a routine's cycles while the CPU is held.
- Save-state layout: unchanged (no encoded field added). With the log disabled and
  `advance_idle` unused, emulation is cycle-identical to upstream (pinned 600-frame baseline
  unchanged).

### `patch(jgenesis): clarify fetch-boundary and GPDMA docs, clear CPU writes on load`

- Files: `backend/snes-core/src/api.rs`, `backend/snes-core/src/memory.rs`.
- Change:
  - `at_fetch_boundary` docs: when it is true, the next tick that does not hold the CPU is the
    opcode fetch. From that state `DmaUnit::tick` can only move HDMA or GPDMA to a pending state
    and return `DmaStatus::None`, so the transfer starts on a later tick.
  - `gpdma_requested` docs (and `CpuInternalRegisters::gpdma_requested`): true while bytes remain
    to be copied; the channel flag clears one DMA tick before the DMA unit is idle.
  - `EmulatorTrait::load_state` keeps only the CPU-write log's `enabled` flag and drops the
    writes recorded before the load (they belong to the discarded timeline).
- Reason: review of the access patch; the oracle drains GPDMA with `!gpdma_requested() &&
  at_fetch_boundary()` and must not see writes from before a load.
- Save-state layout: unchanged. Emulation unchanged (docs, and a non-encoded field).

### `patch(jgenesis): hook detection at opcode fetch`

- Files: `backend/snes-core/src/api.rs`.
- Change:
  - `pub struct HookBitmap`: 2^23 bits, one per physical key `(PBR << 16 | PC) & $7FFFFF`
    (`KEY_MASK`, `key`). `new`, `set`, `clear` and `contains` take any 24-bit `PBR << 16 | PC`
    and drop bank bit 7 (a mirror).
  - `SnesEmulator::set_hooks(Option<Arc<HookBitmap>>)`, which also drops a pending hit, and
    `take_hook_hit() -> Option<u32>`.
  - Detection at the very end of `tick_inner`, after the pending CPU write, the cycle accounting
    and the refresh check. If the tick did not hold the CPU, a bitmap is installed,
    `at_fetch_boundary()` is true and the bitmap contains `PBR << 16 | PC`, the full 24-bit
    address is recorded as the hit. The tick is already complete, so DMA, memory refresh and
    interrupt polling (`execute`, `final_cycle`, `poll_interrupt_lines`) are untouched. The next
    unheld tick fetches the opcode, after which the CPU is mid-instruction: one hit per arrival.
    Held ticks (`advance_idle`) record nothing: the CPU does not move, and re-recording would
    report an arrival that was already taken.
  - Non-encoded `Hooks { bitmap, hit }` (`FakeEncode`/`FakeDecode`, `#[partial_clone(default)]`):
    `load_state` carries the bitmap over and clears the hit; `snapshot()` shares the bitmap
    (`Arc`) and clears the hit; `create`, and so `hard_reset`, starts without hooks.
- Reason: the oracle replaces or verifies original routines at their entry (spec §5.2), so the
  check sits where the CPU is about to fetch the routine's first opcode.
- Save-state layout: unchanged (no encoded field added). With no bitmap installed the only change
  is one `Option` test per unheld tick; emulation is cycle-identical (`BASELINE_600` and
  `FINGERPRINT_600` hold, also with every M1 candidate routine hooked and resumed as original).

### `patch(jgenesis): hook re-detection after oracle-driven moves`

- Files: `backend/snes-core/src/api.rs`, `cpu/wdc65816-emu/src/core.rs`,
  `cpu/wdc65816-emu/src/core/instructions.rs`.
- Change:
  - `SnesEmulator::detect_hook_hit`: the end-of-tick check (bitmap installed,
    `at_fetch_boundary()`, bitmap contains `PBR << 16 | PC`) as a method; the end of every unheld
    tick now calls it (same code, `#[inline(always)]`). `load_state` calls it after the swap, so
    a state at a hooked opcode fetch reports the arrival instead of losing it.
  - `set_hooks` keeps a pending hit whose address the new bitmap still contains
    (`hit.filter(|a| new_bitmap.contains(a))`); otherwise it is dropped as before.
  - `Wdc65816::poll_interrupt_lines` (public wrapper; `instructions::poll_interrupt_lines`
    becomes `pub(super)`) and `SnesEmulator::poll_cpu_interrupt_lines`: sample the NMI edge and
    the IRQ level into the CPU as an instruction's final cycle does.
  - Docs: `advance_idle` states that held ticks record no hit and do not poll, and that the CPU
    must resume a hooked fetch after a hold only from `at_fetch_boundary()` (else the unheld tick
    that completes a pending refresh or DMA records the same arrival again); `take_hook_hit`
    mentions `detect_hook_hit`.
- Reason: the oracle's `Rust` hook mode moves the CPU itself (return emulation through
  `set_cpu_registers`, cycles charged with held ticks). Without a re-check, a return to a hooked
  address is reported only when a refresh or DMA tick happens to follow, and an NMI raised
  during the charge is seen one instruction late. The oracle calls `poll_cpu_interrupt_lines` and
  then `detect_hook_hit` after each Rust call; Verify's demotion reinstalls the table while a hit
  of another routine may be pending (`set_hooks` must keep it).
- Save-state layout: unchanged (no encoded field). Unused, emulation is cycle-identical: the
  end-of-tick check is the same code, `load_state` re-detects only with a bitmap installed,
  `set_hooks` differs only with a pending hit, and nothing calls the poll on its own
  (`BASELINE_600` and `FINGERPRINT_600` hold).

### `patch(jgenesis): slow held tick for exact charging`

- Files: `backend/snes-core/src/api.rs`, `backend/snes-core/src/bus.rs`.
- Change:
  - `tick_inner` gains `hold_slow`: a held CPU cycle lasts `SLOW_MASTER_CYCLES` (8, now
    `pub(crate)` in `bus.rs`) instead of the 6 of `bus.idle()`. `tick` and `advance_idle` pass
    `false`.
  - `SnesEmulator::hold_tick(…, slow)`: one tick with the CPU held; if it is a CPU cycle (no
    refresh or DMA) it lasts 8 master cycles when `slow`, else 6.
- Reason: the oracle charges a Rust routine its cost in CPU-busy master cycles. Held cycles of 6
  alone charge 6·⌈T/6⌉, 2 or 4 master cycles too many for most costs, which shifts the number
  of RNG calls in the game's idle loop every frame. Cycles of 6 and 8 add up to every even
  T ≥ 12, and master-cycle costs are always even.
- Save-state layout: unchanged. Unused, emulation is cycle-identical: `tick` and `advance_idle`
  pass a constant `false` to the `#[inline(always)]` `tick_inner` (`BASELINE_600` and
  `FINGERPRINT_600` hold).

### `patch(jgenesis): expose interrupt handling state`

- Files: `backend/snes-core/src/api.rs`.
- Change: `SnesEmulator::is_mid_instruction` and `SnesEmulator::is_handling_interrupt` forward to
  the `Wdc65816` methods of the same names: the CPU is between two cycles of an instruction or of
  an interrupt entry; the CPU is in an NMI/IRQ entry sequence (BRK and COP are instructions).
- Reason: the oracle's `Verify` mode runs the original routine one tick at a time and must tell
  when it has returned (an instruction boundary with S above its entry value, even with an NMI
  pending) and whether an interrupt entered it (such calls are not compared in M1a).
- Save-state layout: unchanged. Emulation unchanged (read-only accessors).

### `patch(jgenesis): count HDMA transfer units`

- Files: `backend/snes-core/src/memory/dma.rs`, `backend/snes-core/src/api.rs`.
- Change:
  - `DmaUnit` gains a non-encoded counter of HDMA copy units (`HdmaCopyUnits`, `FakeEncode`/
    `FakeDecode`), incremented in `hdma_progress_transfer` after each `hdma_copy_unit` call: one
    per active channel on each scanline that transfers data. Line counter reloads, the frame-start
    reload check and table address loads are not copy units. `DmaUnit::hdma_copy_units` reads it,
    `DmaUnit::carry_over_hdma_copy_units` copies it from another unit.
  - `SnesEmulator::hdma_copy_units` exposes it. `load_state` carries it over from `self` (the
    decoded state has 0); `snapshot()` keeps it through the `DmaUnit` clone (listed there in a
    comment); `create`, and so `hard_reset`, starts at 0.
- Reason: the oracle's `Verify` mode must know whether HDMA wrote anything while the original
  routine ran (`hdma_crossed`: VRAM, CGRAM, OAM and register writes are then not compared).
  `hdma_busy()` is also true for a moment at the start of every frame without any HDMA channel
  (the reload check), so sampling it flagged calls that crossed the frame start; a counter delta
  is exact and needs no per-step sampling.
- Save-state layout: unchanged (the field encodes nothing; `FORMAT_VERSION` stays). Unused,
  emulation is cycle-identical: one counter increment per copy unit, read by nothing in the core
  (`BASELINE_600` and `FINGERPRINT_600` hold).

### `patch(jgenesis): count NMIs`

- Files: `backend/snes-core/src/memory.rs`, `backend/snes-core/src/api.rs`.
- Change:
  - `CpuInternalRegisters` gains a non-encoded NMI counter (`NmiCount`, `FakeEncode`/
    `FakeDecode`), incremented in `acknowledge_nmi`, which the CPU calls once per NMI it takes
    (when it starts the NMI entry sequence; a nested NMI counts too).
    `CpuInternalRegisters::nmi_count` reads it, `carry_over_nmi_count` copies it from another
    instance.
  - `SnesEmulator::nmi_count` exposes it. `load_state` carries it over from `self` (the decoded
    state has 0); `snapshot()` keeps it through the `CpuInternalRegisters` clone (listed there in
    a comment); `create`, and so `hard_reset`, starts at 0. A soft reset keeps it.
- Reason: the oracle's sync points (lockstep comparison of two machines) are the first arrival at
  the game's idle loop after each NMI; NMI deltas between events also reveal lag frames.
- Save-state layout: unchanged (the field encodes nothing; `FORMAT_VERSION` stays). Unused,
  emulation is cycle-identical: one counter increment per NMI, read by nothing in the core
  (`BASELINE_600` and `FINGERPRINT_600` hold).

### `patch(jgenesis): time to the next timing event`

- Files: `backend/snes-core/src/ppu.rs`, `backend/snes-core/src/memory.rs`,
  `backend/snes-core/src/memory/dma.rs`, `backend/snes-core/src/api.rs`.
- Change:
  - `Ppu`: `scanline_length` forwards to `scanline_length_in(scanline, odd_frame)` (same body,
    the parity as a parameter). New: `first_vblank_scanline` (display height + 1),
    `master_cycles_until(scanline, offset)`: master cycles from the current position to the next
    time the PPU reaches `offset` master cycles into `scanline` (an offset past the line's end
    lands in the next line), searched in the previous frame (a point past its end), this frame
    and the next two, with the line lengths of `scanline_length` for each frame's parity and the
    frame lengths of `tick` (one more line in even interlaced frames); a line no frame has gives
    the end of the frame after next (a lower bound). `master_cycles_until_irq_match(htime,
    vtime)`: the same for the IRQ counters, which run `IRQ_OFFSET_MCLKS` (10) behind the
    position: H matches `10 + 4 * HTIME` into a line longer than `4 * HTIME`, V at offset 10 of
    line VTIME, HV both; `u64::MAX` for an HTIME past the longest line or a VTIME past the last.
  - `CpuInternalRegisters::master_cycles_until_interrupt(ppu, irq_enabled)`: 0 if an NMI (or,
    with `irq_enabled`, an IRQ) is pending; else the minimum of the vblank NMI edge (start of
    the first vblank line, if NMI is enabled in NMITIMEN, as `update_hv_blank_flags` raises it)
    and, with `irq_enabled`, the next match of the H, V or HV IRQ mode (as `update_irq_line`
    raises it); `u64::MAX` if none.
  - `DmaUnit::master_cycles_until_hdma(ppu, registers)`: 0 if HDMA is busy or the conditions of
    `check_hdma_reload` / `check_hdma_start` already hold (the next DMA tick starts it);
    `u64::MAX` if no channel is enabled in HDMAEN; else the minimum of the frame-start reload
    (V=0, H=4) and, if a channel is active (enabled, line counter not 0), the next H=276 of a
    line before vblank.
  - `SnesEmulator::master_cycles_until_timing_event()`: the minimum of the two above with
    `irq_enabled` = the CPU's I flag clear; 0 if a cartridge IRQ is pending and the I flag is
    clear. GPDMA (started by the CPU) and future cartridge IRQs are not predicted.
  - Tests in `memory.rs`: from a sweep of positions over several frames (NTSC, overscan,
    interlaced, PAL), the predicted NMI and H/V/HV IRQ times are exactly when the registers
    raise them when ticked (2 and 4 master-cycle resolution).
- Known limit (documented, not modelled): with interlace on and VTIME = scanlines per frame − 1, a
  second V-IRQ edge can come at line 0, offset 0–9 after an even frame, which
  `master_cycles_until_timing_event` does not predict (the first edge, at offset 10 of line
  VTIME, is).
- Reason: the oracle's `Rust` mode runs a Rust routine (atomic at its entry, then charged its
  cost with the CPU held) only when no timing event can land inside the original routine: an
  NMI or IRQ taken mid-routine or HDMA stealing cycles at a CPU-cycle-dependent phase cannot be
  reproduced, and lockstep with the original must stay cycle-exact.
- Save-state layout: unchanged (no field added). Emulation unchanged: read-only accessors, and
  `scanline_length` computes the same value (`BASELINE_600` and `FINGERPRINT_600` hold).

### `patch(jgenesis): fetch boundary when the DMA unit has nothing to do`

- Files: `backend/snes-core/src/memory/dma.rs`, `backend/snes-core/src/api.rs`.
- Change: `DmaUnit::lets_cpu_run(registers)`: the next `tick` takes no DMA cycle (no DMA cycles
  in progress; HDMA idle, or reload-pending with no channel enabled in HDMAEN, or start-pending
  with no active channel; GPDMA idle, or pending with no channel enabled in MDMAEN: the states
  `hdma_reload`, `hdma_start` and `gpdma_start` leave without a cycle), true whenever
  `DmaUnit::is_idle` is. `at_fetch_boundary` uses it instead of `is_idle`.
- Reason: with no HDMA channel enabled, the frame-start reload check leaves HDMA reload-pending
  for one tick; the next tick clears it and fetches the opcode. `at_fetch_boundary` was false in
  between, so an arrival at a hooked address then was never reported. For a routine this only
  meant one call run as original, but the oracle's `call_original` returns to a trap address
  that must be reported, or the CPU executes the open bus there.
- Visible changes, all at an opcode fetch that coincides with such a no-op DMA tick (at most one
  per frame for the reload check, when no HDMA channel is enabled): the arrival at a hooked
  address is now reported, so a `Verify` routine gets one more checked (or sampled) call, a
  `Rust` routine one more dispatch (a `timing_fallback`, since HDMA counts as busy then), and a
  sync point an arrival that can make a sync event one idle-loop iteration earlier; the oracle's
  GPDMA drain (held until `at_fetch_boundary`) ends one held CPU cycle earlier when its transfer
  ends there.
- Save-state layout: unchanged. Emulation unchanged: `at_fetch_boundary` only drives hook
  detection, which runs only with a hook bitmap installed (`BASELINE_600` and `FINGERPRINT_600`
  hold).

### `patch(jgenesis): test the DMA states that let the CPU run`

- Files: `backend/snes-core/src/memory/dma.rs` (tests only).
- Change: unit tests for `DmaUnit::lets_cpu_run`: HDMA start-pending (no active channel), HDMA
  reload-pending (no enabled channel) and GPDMA pending (no enabled channel) let the CPU run, the
  next `tick` returning `DmaStatus::None`; with a channel to serve they take the bus.
- Reason: the previous patch relies on those three states, and `lets_cpu_run` must say exactly
  when a tick takes no DMA cycle.
- Save-state layout: unchanged. Emulation unchanged (tests only).

### `patch(jgenesis): move the ROM Arc on state load and hard reset`

- Files: `backend/snes-core/src/memory/cartridge.rs`, `backend/snes-core/src/memory.rs`,
  `backend/snes-core/src/api.rs`.
- Change:
  - `RomImage` (`Bytes(Vec<u8>)` or `Shared(Arc<[u8]>)`): `Cartridge::create` and `Memory::create`
    take `impl Into<RomImage>`, so a `Vec<u8>` still works at every existing call site. A
    `Shared` image is the already normalised ROM of a plain mapper (no copier header, a power of
    two long): header stripping and mirroring are skipped and the mapper's `Rom` is that `Arc`;
    the mappers with a box of their own (CX4, OBC1, SA-1, S-DD1, SPC7110, Super FX) copy it into
    one.
  - `Cartridge::take_rom_from` (used by `load_state`) moves the `Arc` between two plain mappers
    (`LoRom`, `HiRom`, `ExHiRom`, `DspLoRom`, `DspHiRom`, `St01x`, `St018`) instead of copying
    the ROM into a `Vec` and from there into a new `Arc` (two copies of the image per load); the
    other mappers keep the copying path.
  - `Cartridge::take_rom_image` and `Memory::take_rom_image` replace `Memory::take_rom`: `hard_reset`
    starts again from the shared image through `SnesEmulator::create_from_image` (private;
    `SnesEmulator::create` keeps its signature and forwards to it).
  - Tests in `memory/cartridge.rs`: a state load and a hard reset keep the same allocation, a
    ROM given as bytes is normalised as before.
- Reason: loading a state (F9, `repro`, the oracle's `load_state`) and a hard reset copied the
  whole ROM twice for no purpose.
- Save-state layout: unchanged. Emulation identical (`BASELINE_600` and `FINGERPRINT_600` hold).

### `patch(jgenesis): test the HDMA prediction`

- Files: `backend/snes-core/src/memory/dma.rs` (tests only).
- Change: `time_to_hdma` steps a PPU and a DMA unit together, 4 master cycles at a time, with HDMA
  channel 0 reading a table from the ROM (254 lines of one byte to COLDATA, or 2 lines and the
  end, so the channel goes inactive), over NTSC, overscan, interlaced, PAL and PAL overscan
  frames. At 700 positions per configuration, spread by a sweep, `DmaUnit::master_cycles_until_hdma`
  must be exactly the master cycles until the DMA unit takes the bus (the frame-start reload and
  the H=276 transfer of each display line, none in vblank), and 0 only when the next DMA tick
  starts it. The DMA unit runs on to idle after each event with the PPU standing still.
- Reason: the NMI and IRQ predictions had unit tests against real stepping, the HDMA one only
  the oracle's lockstep runs.
- Save-state layout: unchanged. Emulation unchanged (tests only).

### `patch(jgenesis): per-scanline render log for a renderer outside the core`

- Files: `backend/snes-core/src/ppu/linelog.rs` (new), `backend/snes-core/src/ppu.rs`,
  `backend/snes-core/src/ppu/sprites.rs`, `backend/snes-core/src/api.rs`,
  `backend/snes-core/src/lib.rs`.
- Change: `Ppu` gains an output-only recorder, off by default (`Ppu::set_line_log`,
  `SnesEmulator::set_ppu_line_log`). While on, every `render_current_line` appends a `LineEntry`
  (a `LineRegs` copy of the registers the render reads as plain values, the Mode 7 latch, the
  mosaic's vertical offset, and a `SpriteLine` with the OBSEL values the sprites of that line were
  evaluated and fetched with, and whether their line buffer was rebuilt). At the start of a frame
  (the scanline counter wrapping to 0) CGRAM, VRAM and OAM are copied; the writes to CGRAM and VRAM
  while the frame is displayed (HDMA gradients; the VRAM and OAM loads the game does in the forced
  blanking it toggles mid-frame) are listed with the number of entries rendered before them. When
  the display ends (the tick that reports the frame) the frame
  is complete. `SnesEmulator::ppu_frame_log` gives the last completed frame's `FrameLog`
  (`snes_core::linelog`). The recorder is a `FakeEncode` field: not in save states, not compared;
  `load_state` and `hard_reset` carry the setting over, and `snapshot` keeps it without the log.
- Reason: M6 (HD graphics) re-renders each frame outside the core from VRAM, OAM, CGRAM and the
  per-line registers; the frame-end state alone
  cannot give the per-line HDMA effects (OBSEL, TM, scroll, CGRAM, fixed color, windows).
- Save-state layout: unchanged. Emulation identical: nothing the recorder holds is read by the PPU,
  the CPU or the DMA (`verify-replays` and the replay re-recording show it).

### `patch(jgenesis): output switches and cheaper idle ticks for fast-forward`

- Files: `backend/snes-core/src/ppu.rs`, `backend/snes-core/src/ppu/sprites.rs`,
  `backend/snes-core/src/audio.rs`, `backend/snes-core/src/api.rs`,
  `backend/snes-core/src/memory.rs`, `backend/snes-core/src/memory/dma.rs`,
  `backend/snes-core/src/memory/cartridge.rs`, `backend/snes-core/src/memory/inputs.rs`.
- Change:
  - `Ppu::set_compose_pixels` / `SnesEmulator::set_ppu_compose_pixels` (default on): with it off,
    `render_current_line` and the object line buffer (`sprites_finish_line`) draw nothing for a
    line of a 256-pixel progressive frame; the render log, the sprite evaluation and tile fetch
    (range and time-over flags), counters and latches are untouched. Hi-res, pseudo-hi-res and
    interlaced frames are always drawn. The setting is a `FakeEncode` field, carried over by
    `load_state`, `hard_reset` and `snapshot`.
  - `AudioResampler` gains an enabled switch (`SnesEmulator::set_audio_resampling`, default on,
    `FakeEncode`, carried over the same way): with it off `collect_sample` does nothing, so the DSP
    still runs but no sample is filtered or resampled. `output_samples` is now called only after a
    collected sample (the queue is empty otherwise).
  - `SnesEmulator::ppu_line_log_enabled` accessor.
  - Per-tick shortcuts with identical results: `Ppu::tick` returns early when the tick ends before
    the next dot event, the end of the line and any mid-line update (the event table is hoisted
    to a module constant); `update_irq_line` with IRQs off only stores the dot and clears the line;
    `check_hdma_start` tests the position before scanning the channels; `gpdma_requested` tests
    the eight flags as one word (a `bytemuck` cast of `[bool; 8]`, which needs the workspace's
    `bytemuck` feature `min_const_generics`); `Cartridge::tick`, `Memory::tick`, `InputState::tick`,
    `tick_cpu_cycle` and `CpuInternalRegisters::tick` are inlined.
- Reason: fast-forward (second performance pass): frames of a batch that are not shown need no
  pixels, no render log and no resampled sound, and the per-tick work of the core is the rest of
  the emulation cost.
- Save-state layout: unchanged. Emulation identical with the switches on; with them off everything
  the CPU can observe is identical (`xtask hd-skip-check` plays whole replays both ways and compares
  the PPU frames of the shown frames, the work RAM, the registers and the master cycles).
