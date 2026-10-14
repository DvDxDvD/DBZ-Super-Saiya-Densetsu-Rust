//! What generated code runs on: the register file of a recompiled routine ([`R`]), its memory
//! and stack access with the hardware's cycle charging, and the 65816 arithmetic.
//!
//! A recompiled function works on an [`R`] loaded from the machine at the routine's entry and
//! stored back when it ends. Memory goes through the oracle's [`Ctx`] bus access, so the
//! hardware registers, DMA, WRAM and the mirrors behave as in the core. Time is kept as pending
//! master cycles in [`R::cy`] and charged to the machine ([`R::flush`]) before an access to the
//! hardware registers and at the end, so every such access lands at the cycle the original makes
//! it (reads at the start of their cycle, writes at its end, as the core orders them).
//!
//! Before each stretch of code (a basic block) the function checks that no timing event can come
//! within the stretch's worst-case cost ([`R::gate`]); if one can, the function stops and hands
//! control back to the original code at that address (`Ctx::resume_at`), which is exact because
//! the registers and the stack are always as the hardware has them there.

use ssd_oracle::{Ctx, StatusRegister};

/// Returned by a generated function that stopped before its return: the original code goes on at
/// [`R::resume`].
pub const BAIL: u32 = u32::MAX;

/// The most master cycles that pass between two window checks (see `R::gate`).
const MAX_SLACK: i64 = 100_000;

/// Debugging (`xtask fuzz-recomp ... --repro`): a generated routine hands over at the first block
/// boundary at or after this many instructions, so that its state can be compared with the
/// original's after the same number of instructions. `u64::MAX`: never.
pub static STOP_AFTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// The most nested generated calls (tail calls included) before a function hands over: Rust has
/// no tail calls, and the original loops by jumping.
pub const MAX_DEPTH: u32 = 150;
/// The most a routine may push below its entry's S for the bytes to count as dead stack.
const MAX_PUSH_DEPTH: u16 = 0x200;

/// The master cycles a stretch costing `cost` CPU-busy master cycles can span (the window of
/// `Rust` mode, see `Ctx::window`).
#[must_use]
pub const fn win(cost: u32) -> u32 {
    let refreshes = (cost + 1360) / 1320;
    cost + refreshes * 40 + 8
}

/// The master cycles the CPU spends on a bus cycle at `ea`: 6 for the system registers
/// `$2000-$3FFF` and `$4200-$5FFF`, 12 for `$4000-$41FF` (banks `$00-$3F` and `$80-$BF`), 8 for
/// everything else (this ROM is `SlowROM`, and never writes `MEMSEL`).
#[inline(always)]
#[must_use]
pub fn spd(ea: u32) -> u32 {
    if ea & 0x40_0000 != 0 {
        return 8;
    }
    match ea as u16 {
        0x2000..=0x3FFF | 0x4200..=0x5FFF => 6,
        0x4000..=0x41FF => 12,
        _ => 8,
    }
}

/// The two-bit code of a cycle duration.
#[inline(always)]
fn code_of(duration: u32) -> u8 {
    match duration {
        6 => 0,
        8 => 1,
        _ => 2,
    }
}

/// Whether an access to `ea` can touch hardware whose state depends on the time of the access
/// (the registers of the PPU, APU, CPU and DMA).
#[inline(always)]
#[must_use]
pub fn is_io(ea: u32) -> bool {
    ea & 0x40_0000 == 0 && matches!(ea as u16, 0x2000..=0x5FFF)
}

/// Whether a read at `ea` can return the open bus: the system area outside the work RAM mirror
/// and the registers' range (`$2000-$7FFF` of the banks `$00-$3F` and `$80-$BF`). The open bus
/// is the last byte read from the cartridge, which a recompiled instruction does not fetch.
#[inline(always)]
#[must_use]
pub fn is_open(ea: u32) -> bool {
    ea & 0x40_0000 == 0 && matches!(ea as u16, 0x2000..=0x7FFF)
}

/// Cycle penalty of an indexed read, 8-bit index: one idle cycle if the index crosses a page.
#[inline(always)]
#[must_use]
pub fn pcx(base: u16, index: u16) -> u32 {
    if (base.wrapping_add(index) ^ base) & 0xFF00 != 0 { 6 } else { 0 }
}

static CHAOS_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `SSD_RECOMP_CHAOS=N` (debugging): every N-th window check fails, so the code hands over at
/// block boundaries far more often than the timing requires.
fn chaos() -> Option<u64> {
    static CHAOS: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *CHAOS.get_or_init(|| {
        std::env::var("SSD_RECOMP_CHAOS").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0)
    })
}

/// `SSD_RECOMP_TRACE=1` (debugging): every end of a recompiled routine is printed.
fn trace() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("SSD_RECOMP_TRACE").is_some())
}

/// `SSD_RECOMP_STATS=1` (debugging): the hand-overs are counted by address and cause
/// ([`bail_stats`]).
fn stats_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SSD_RECOMP_STATS").is_some())
}

static BAIL_STATS: std::sync::Mutex<Vec<((u32, &'static str), u64)>> =
    std::sync::Mutex::new(Vec::new());

fn count_bail(pc: u32, why: &'static str) {
    let mut stats = BAIL_STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match stats.iter_mut().find(|(k, _)| *k == (pc, why)) {
        Some((_, n)) => *n += 1,
        None => stats.push(((pc, why), 1)),
    }
}

/// The hand-overs counted since the start (needs `SSD_RECOMP_STATS=1`): `(address, cause)` and
/// how often.
#[must_use]
pub fn bail_stats() -> Vec<((u32, &'static str), u64)> {
    BAIL_STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The registers and bookkeeping of a recompiled routine.
#[derive(Debug, Clone)]
pub struct R {
    pub a: u16,
    pub x: u16,
    pub y: u16,
    pub s: u16,
    pub d: u16,
    pub dbr: u8,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// The decimal flag.
    pub dec: bool,
    /// The interrupt-disable flag.
    pub irq: bool,
    /// The M and X flags as in P (`0x20`, `0x10`): kept up to date at the places the state is
    /// observed (a stop, a return).
    pub mx: u8,
    /// Master cycles spent and not yet charged to the machine.
    pub cy: u32,
    /// The durations of the last `hn` cycles in `cy`, two bits each (0: 6, 1: 8, 2: 12), the
    /// most recent in the low bits: they are charged one tick each, so the tick boundaries
    /// that matter (a refresh, the sound CPU clock catching up) fall where the original has
    /// them. Earlier cycles are charged in bulk, whose tick boundaries are of no consequence.
    pub hist: u8,
    pub hn: u8,
    /// Master cycles charged to the machine since the entry.
    pub charged: u64,
    /// Master cycles that can still pass before the next window check.
    pub slack: i64,
    /// Instructions executed since the entry.
    pub instrs: u64,
    /// Where the original goes on after a [`BAIL`].
    pub resume: u32,
    /// Nesting of generated calls.
    pub depth: u32,
    /// Running on the snapshot of a `Verify` call: no timing windows.
    pub nogate: bool,
    /// A frame was rendered during the routine: it hands over at the next window check.
    pub stop: bool,
    /// The interrupt lines were sampled before the last write.
    pub nopoll: bool,
    /// The entry the routine was called at (tracing).
    pub entry: u32,
    /// Why the last window check failed (debugging statistics).
    pub why: &'static str,
    /// The block a function starts at (0 for its entry; set by [`run_mid`]).
    pub start: u32,
    /// How the last generated return returned: with RTL (24-bit address) or RTS.
    pub rl: bool,
    /// The lowest stack pointer the routine pushed down to.
    pub low: u16,
    /// The stack pointer at the start.
    pub s0: u16,
    /// Instructions after which the routine hands over ([`STOP_AFTER`]).
    pub stop_after: u64,
    /// Instructions after which a Verify snapshot's run counts as a runaway.
    pub limit: u64,
    /// A pull other than a return took the stack pointer above the start: the routine drops
    /// its caller's return address (checked on a Verify snapshot only).
    pub dropped: bool,
}

impl R {
    /// The registers of the machine.
    #[must_use]
    pub fn load(ctx: &Ctx<'_>) -> Self {
        let g = ctx.registers();
        let p = u8::from(g.p);
        Self {
            a: g.a,
            x: g.x,
            y: g.y,
            s: g.s,
            d: g.d,
            dbr: g.dbr,
            n: p & 0x80 != 0,
            z: p & 0x02 != 0,
            c: p & 0x01 != 0,
            v: p & 0x40 != 0,
            dec: p & 0x08 != 0,
            irq: p & 0x04 != 0,
            mx: p & 0x30,
            cy: 0,
            hist: 0,
            hn: 0,
            charged: 0,
            slack: 0,
            instrs: 0,
            resume: 0,
            depth: 0,
            nogate: ctx.on_verify_snapshot(),
            stop: false,
            nopoll: false,
            entry: 0,
            why: "",
            start: 0,
            rl: false,
            low: g.s,
            s0: g.s,
            stop_after: STOP_AFTER.load(std::sync::atomic::Ordering::Relaxed),
            limit: ctx.runaway_limit(),
            dropped: false,
        }
    }

    /// P with the widths `mx`.
    #[inline(always)]
    #[must_use]
    pub fn p(&self, mx: u8) -> u8 {
        (u8::from(self.n) << 7)
            | (u8::from(self.v) << 6)
            | mx
            | (u8::from(self.dec) << 3)
            | (u8::from(self.irq) << 2)
            | (u8::from(self.z) << 1)
            | u8::from(self.c)
    }

    /// Takes the flags of a pulled P; returns its widths. With 8-bit index registers their high
    /// bytes are cleared, as the CPU does.
    #[inline(always)]
    pub fn set_p(&mut self, p: u8) -> u8 {
        self.n = p & 0x80 != 0;
        self.v = p & 0x40 != 0;
        self.dec = p & 0x08 != 0;
        self.irq = p & 0x04 != 0;
        self.z = p & 0x02 != 0;
        self.c = p & 0x01 != 0;
        if p & 0x10 != 0 {
            self.x &= 0x00FF;
            self.y &= 0x00FF;
        }
        p & 0x30
    }

    // Time

    /// Adds `n` master cycles: `k` cycles (at most 3) end the run, with durations `h` (two bits
    /// each, the last in the low bits).
    #[inline(always)]
    pub fn add(&mut self, n: u32, k: u32, h: u32) {
        self.cy += n;
        self.hist = (((u32::from(self.hist) << (2 * k)) | h) & 0x3F) as u8;
        self.hn = (u32::from(self.hn) + k).min(3) as u8;
    }

    /// Adds a penalty cycle (`n` is 0 or 6).
    #[inline(always)]
    pub fn addp(&mut self, n: u32) {
        if n != 0 {
            self.cy += n;
            self.push(0);
        }
    }

    #[inline(always)]
    fn push(&mut self, code: u8) {
        self.hist = ((self.hist << 2) | code) & 0x3F;
        self.hn = (self.hn + 1).min(3);
    }

    /// Charges the pending cycles to the machine.
    #[inline(always)]
    pub fn flush(&mut self, ctx: &mut Ctx<'_>) {
        if self.cy != 0 {
            self.charge(ctx, false);
        }
    }

    /// Charges the pending cycles: the older ones in bulk, the last few tick by tick. With
    /// `poll`, the interrupt lines are sampled before the last tick.
    #[inline(never)]
    fn charge(&mut self, ctx: &mut Ctx<'_>, poll: bool) {
        let n = usize::from(self.hn);
        let mut ticks = [0_u64; 3];
        let mut tail = 0_u64;
        for (i, tick) in ticks.iter_mut().enumerate().take(n) {
            let code = (self.hist >> (2 * (n - 1 - i))) & 3;
            *tick = [6, 8, 12][usize::from(code)];
            tail += *tick;
        }
        let cy = u64::from(self.cy);
        // The remainder must be a time the machine can hold exactly (0, 6, 8, or an even
        // number from 12 up); if the recorded ticks are not a true suffix, charge in bulk.
        let valid = |t: u64| matches!(t, 0 | 6 | 8) || (t >= 12 && t.is_multiple_of(2));
        let (bulk, n) = if cy >= tail && valid(cy - tail) { (cy - tail, n) } else { (cy, 0) };
        if bulk > 0 {
            ctx.charge(bulk);
        }
        for (i, &tick) in ticks.iter().enumerate().take(n) {
            if poll && i + 1 == n {
                ctx.poll_lines();
            }
            ctx.charge(tick);
        }
        if poll && n == 0 {
            ctx.poll_lines();
        }
        self.charged += cy;
        self.cy = 0;
        self.hn = 0;
        self.hist = 0;
        if ctx.frame_rendered() {
            // A frame ended: the machine loop checks its stops (the end of a slice of
            // frames) when the routine hands over at the next window check.
            self.slack = 0;
            self.stop = true;
        }
    }

    /// The check before a stretch that costs at most `cost` master cycles (`wcost` = [`win`] of
    /// it) and runs `n` instructions: true if no timing event can come within it.
    #[inline(always)]
    pub fn gate(&mut self, ctx: &mut Ctx<'_>, wcost: u32, n: u32) -> bool {
        if self.slack > i64::from(wcost) {
            self.slack -= i64::from(wcost);
            self.instrs += u64::from(n);
            return true;
        }
        self.regate(ctx, wcost, n)
    }

    #[cold]
    #[inline(never)]
    fn regate(&mut self, ctx: &mut Ctx<'_>, wcost: u32, n: u32) -> bool {
        if let Some(period) = chaos() {
            // Debugging: hand over at every `period`-th window check, to test the hand-over.
            let count = CHAOS_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if count % period == period - 1 && !self.nogate {
                return false;
            }
            let ok = self.regate_inner(ctx, wcost, n);
            self.slack = 0;
            return ok;
        }
        self.regate_inner(ctx, wcost, n)
    }

    fn regate_inner(&mut self, ctx: &mut Ctx<'_>, wcost: u32, n: u32) -> bool {
        if self.instrs >= self.stop_after {
            self.why = "stop after";
            return false;
        }
        let ok = self.regate_checked(ctx, wcost, n);
        if self.stop_after != u64::MAX {
            // Every block is checked, so that the count is exact.
            self.slack = 0;
        }
        ok
    }

    fn regate_checked(&mut self, ctx: &mut Ctx<'_>, wcost: u32, n: u32) -> bool {
        if self.nogate {
            // A Verify snapshot has no window, but a call that runs on past any real one is
            // an endless loop: give up and say so.
            if self.dropped {
                self.why = "drops the caller's return";
                ctx.mark_dropped();
                return false;
            }
            if self.instrs > self.limit {
                self.why = "runaway";
                ctx.mark_runaway();
                return false;
            }
            self.slack = 1 << 20;
            self.instrs += u64::from(n);
            return true;
        }
        self.flush(ctx);
        if self.stop {
            self.why = "frame end";
            return false;
        }
        ctx.set_irq_disable(self.irq);
        let until = ctx.cycles_until_event();
        if until <= u64::from(wcost) {
            self.why = "timing event";
            return false;
        }
        if ctx.gpdma_requested() || ctx.failed() {
            self.why = "gpdma";
            return false;
        }
        // At most a stretch of `MAX_SLACK` before the next check, so that time is charged (and
        // a frame boundary seen) even in a loop nothing interrupts.
        self.slack = i64::try_from(until - u64::from(wcost)).unwrap_or(i64::MAX).min(MAX_SLACK);
        self.instrs += u64::from(n);
        true
    }

    /// Hands over to the original at `pc` (24-bit) with widths `mx`.
    #[cold]
    #[inline(never)]
    #[must_use]
    pub fn bail(&mut self, pc: u32, mx: u8) -> u32 {
        if stats_on() {
            count_bail(pc, std::mem::take(&mut self.why).max_empty());
        }
        self.resume = pc;
        self.mx = mx;
        BAIL
    }

    /// Hands over after a call that came back to another address than its caller's: `ret` is
    /// what the return pulled, `bank` the program bank of the caller.
    #[cold]
    #[inline(never)]
    #[must_use]
    pub fn bail_ret(&mut self, ret: u32, long: bool, bank: u32) -> u32 {
        let bank = if long { ret >> 16 } else { bank };
        self.resume = (bank << 16) | (ret.wrapping_add(1) & 0xFFFF);
        BAIL
    }

    /// Hands over at `pc` with the widths the registers have now (after a call or a pull).
    #[cold]
    #[inline(never)]
    #[must_use]
    pub fn bail_dyn(&mut self, pc: u32) -> u32 {
        self.resume = pc;
        BAIL
    }

    /// Ends the routine: stores the registers to the machine and tells it how the routine ended
    /// (`ret` is what the generated function returned; `pulls` the bytes its return pulled, 2
    /// for RTS and 3 for RTL).
    pub fn finish(&mut self, ctx: &mut Ctx<'_>, ret: u32, pulls: u16) {
        self.flush(ctx);
        if trace() {
            eprintln!(
                "recomp: finish {:06X} ret={ret:#X} resume={:06X} master={} charged={} instrs={}",
                self.entry,
                self.resume,
                ctx.total_master_cycles(),
                self.charged,
                self.instrs
            );
        }
        let mut g = ctx.registers();
        // The bytes pushed below the entry's S are dead stack for the comparison with the
        // original (a stack switch, which moves S far, is left out).
        if g.s.wrapping_sub(self.low) <= MAX_PUSH_DEPTH {
            ctx.note_lowest_s(self.low);
        }
        g.a = self.a;
        g.x = self.x;
        g.y = self.y;
        g.d = self.d;
        g.dbr = self.dbr;
        g.p = StatusRegister::from(self.p(self.mx));
        if ret == BAIL {
            g.s = self.s;
            ctx.set_registers(g);
            ctx.resume_at(self.resume);
        } else {
            // The generated return pulled the address; the machine's own return pulls it again.
            g.s = self.s.wrapping_sub(pulls);
            ctx.set_registers(g);
        }
        if self.dropped {
            ctx.mark_dropped();
        }
        ctx.set_cost(self.charged);
        ctx.set_instructions(self.instrs);
        ctx.mark_gated();
        if self.nopoll {
            ctx.suppress_final_poll();
        }
    }

    // Memory

    #[inline(always)]
    pub fn rd8(&mut self, ctx: &mut Ctx<'_>, ea: u32) -> u8 {
        if is_io(ea) {
            self.flush(ctx);
            ctx.settle();
        }
        let v = ctx.read8(ea);
        let s = spd(ea);
        self.cy += s;
        self.push(code_of(s));
        v
    }

    /// A word at `ea`, the high byte at the next 24-bit address.
    #[inline(always)]
    pub fn rd16(&mut self, ctx: &mut Ctx<'_>, ea: u32) -> u16 {
        let lo = self.rd8(ctx, ea);
        let hi = self.rd8(ctx, ea.wrapping_add(1) & 0xFF_FFFF);
        u16::from_le_bytes([lo, hi])
    }

    /// A word of bank `ea >> 16` whose high byte wraps within the 16-bit offset (direct page,
    /// stack, pointers).
    #[inline(always)]
    pub fn rd16w(&mut self, ctx: &mut Ctx<'_>, ea: u32) -> u16 {
        let lo = self.rd8(ctx, ea);
        let hi = self.rd8(ctx, (ea & 0xFF_0000) | (ea.wrapping_add(1) & 0xFFFF));
        u16::from_le_bytes([lo, hi])
    }

    /// A 24-bit pointer of bank `ea >> 16`, offsets wrapping within 16 bits.
    #[inline(always)]
    pub fn rd24w(&mut self, ctx: &mut Ctx<'_>, ea: u32) -> u32 {
        let lo = self.rd8(ctx, ea);
        let mid = self.rd8(ctx, (ea & 0xFF_0000) | (ea.wrapping_add(1) & 0xFFFF));
        let hi = self.rd8(ctx, (ea & 0xFF_0000) | (ea.wrapping_add(2) & 0xFFFF));
        u32::from(lo) | (u32::from(mid) << 8) | (u32::from(hi) << 16)
    }

    #[inline(always)]
    pub fn wr8(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u8) {
        let s = spd(ea);
        self.cy += s;
        self.push(code_of(s));
        if is_io(ea) {
            self.flush(ctx);
            self.slack = 0;
        }
        ctx.write8(ea, v);
    }

    /// A write to one of the CPU's interrupt and timer registers (`$4200-$420A`): the lines are
    /// sampled just before the write, as the last cycle of the instruction does, and the caller
    /// hands over at the next instruction so the original recognises an interrupt the write
    /// raises after it. Returns whether `ea` is such a register.
    #[inline(always)]
    pub fn wr8_ctl(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u8) -> bool {
        if self.nogate {
            self.wr8(ctx, ea, v);
            return false;
        }
        let ctl = (ea >> 16) as u8 & 0x7F <= 0x3F && matches!(ea as u16, 0x4200..=0x420A);
        let s = spd(ea);
        self.cy += s;
        self.push(code_of(s));
        if ctl {
            self.charge(ctx, true);
            self.slack = 0;
            self.nopoll = true;
        } else if is_io(ea) {
            self.flush(ctx);
            self.slack = 0;
        }
        ctx.write8(ea, v);
        ctl
    }

    /// A write to `$420B` (MDMAEN) or wherever the code stores through the same instruction: the
    /// write lands at the end of its cycle and a GPDMA it starts is left to the original code
    /// (the hardware runs it after one more CPU cycle). Returns whether one was started: the
    /// caller then hands over at the next instruction.
    #[inline(always)]
    pub fn wr8_dma(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u8, next: u32) -> bool {
        if self.nogate {
            // On a `Verify` snapshot the transfer runs at once (timing is not compared). The
            // original fetches the next opcode before the transfer starts, which leaves the
            // open bus a transfer from an unreadable source reads.
            if next != u32::MAX && v != 0 && (ea >> 16) as u8 & 0x7F <= 0x3F && ea as u16 == 0x420B
            {
                ctx.read8(next);
            }
            self.wr8(ctx, ea, v);
            return false;
        }
        let s = spd(ea);
        self.cy += s;
        self.push(code_of(s));
        if is_io(ea) {
            self.flush(ctx);
            self.slack = 0;
        }
        ctx.write8_raw(ea, v);
        v != 0 && (ea >> 16) as u8 & 0x7F <= 0x3F && ea as u16 == 0x420B
    }

    /// A write whose address is computed (a pointer or an index) and can be one of the CPU's
    /// event registers (`$4200-$420B`): it is made as [`R::wr8_ctl`] or [`R::wr8_dma`] would, and
    /// the caller hands over at the next instruction if this returns true.
    #[inline(always)]
    pub fn wr8_sys(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u8) -> bool {
        if (ea >> 16) as u8 & 0x7F <= 0x3F && matches!(ea as u16, 0x4200..=0x420B) {
            return self.wr8_sys_slow(ctx, ea, v);
        }
        self.wr8(ctx, ea, v);
        false
    }

    /// A word store that can reach the event registers: the low byte is written as usual, the
    /// high byte (the last cycle of the instruction) as [`R::wr8_sys`]; `wrap` keeps the high
    /// byte within the 16-bit offset.
    #[inline(always)]
    pub fn wr16_sys(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u16, wrap: bool) -> bool {
        self.wr8(ctx, ea, v as u8);
        let hi = if wrap {
            (ea & 0xFF_0000) | (ea.wrapping_add(1) & 0xFFFF)
        } else {
            ea.wrapping_add(1) & 0xFF_FFFF
        };
        self.wr8_sys(ctx, hi, (v >> 8) as u8)
    }

    #[cold]
    #[inline(never)]
    fn wr8_sys_slow(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u8) -> bool {
        if ea as u16 == 0x420B {
            self.wr8_dma(ctx, ea, v, u32::MAX)
        } else {
            self.wr8_ctl(ctx, ea, v)
        }
    }

    /// A word, low byte first, the high byte at the next 24-bit address.
    #[inline(always)]
    pub fn wr16(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u16) {
        self.wr8(ctx, ea, v as u8);
        self.wr8(ctx, ea.wrapping_add(1) & 0xFF_FFFF, (v >> 8) as u8);
    }

    /// A word, low byte first, the high byte wrapping within the 16-bit offset.
    #[inline(always)]
    pub fn wr16w(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u16) {
        self.wr8(ctx, ea, v as u8);
        self.wr8(ctx, (ea & 0xFF_0000) | (ea.wrapping_add(1) & 0xFFFF), (v >> 8) as u8);
    }

    /// A word, high byte first (the write-back of a read-modify-write).
    #[inline(always)]
    pub fn wr16r(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u16) {
        self.wr8(ctx, ea.wrapping_add(1) & 0xFF_FFFF, (v >> 8) as u8);
        self.wr8(ctx, ea, v as u8);
    }

    /// As [`R::wr16r`], the high byte wrapping within the 16-bit offset.
    #[inline(always)]
    pub fn wr16rw(&mut self, ctx: &mut Ctx<'_>, ea: u32, v: u16) {
        self.wr8(ctx, (ea & 0xFF_0000) | (ea.wrapping_add(1) & 0xFFFF), (v >> 8) as u8);
        self.wr8(ctx, ea, v as u8);
    }

    // Addresses

    /// Direct page `D + offset + index` (bank 0).
    #[inline(always)]
    #[must_use]
    pub fn dp(&self, offset: u8, index: u16) -> u32 {
        u32::from(self.d.wrapping_add(u16::from(offset)).wrapping_add(index))
    }

    /// The extra cycle when the low byte of D is not zero.
    #[inline(always)]
    #[must_use]
    pub fn dpx(&self) -> u32 {
        if self.d as u8 != 0 { 6 } else { 0 }
    }

    /// `DBR:base + index`, carrying into the bank.
    #[inline(always)]
    #[must_use]
    pub fn db(&self, base: u16, index: u16) -> u32 {
        ((u32::from(self.dbr) << 16) + u32::from(base) + u32::from(index)) & 0xFF_FFFF
    }

    // Stack

    #[inline(always)]
    pub fn push8(&mut self, ctx: &mut Ctx<'_>, v: u8) {
        self.wr8(ctx, u32::from(self.s), v);
        self.s = self.s.wrapping_sub(1);
        self.low = self.low.min(self.s);
    }

    /// High byte first.
    #[inline(always)]
    pub fn push16(&mut self, ctx: &mut Ctx<'_>, v: u16) {
        self.push8(ctx, (v >> 8) as u8);
        self.push8(ctx, v as u8);
    }

    #[inline(always)]
    pub fn pull8(&mut self, ctx: &mut Ctx<'_>) -> u8 {
        self.s = self.s.wrapping_add(1);
        if self.nogate && self.s > self.s0 {
            self.dropped = true;
            self.slack = 0;
        }
        self.rd8(ctx, u32::from(self.s))
    }

    #[inline(always)]
    pub fn pull16(&mut self, ctx: &mut Ctx<'_>) -> u16 {
        let lo = self.pull8(ctx);
        let hi = self.pull8(ctx);
        u16::from_le_bytes([lo, hi])
    }

    /// A pull by a return: it is the routine's own return address.
    #[inline(always)]
    pub fn pull8r(&mut self, ctx: &mut Ctx<'_>) -> u8 {
        self.s = self.s.wrapping_add(1);
        self.rd8(ctx, u32::from(self.s))
    }

    #[inline(always)]
    pub fn pull16r(&mut self, ctx: &mut Ctx<'_>) -> u16 {
        let lo = self.pull8r(ctx);
        let hi = self.pull8r(ctx);
        u16::from_le_bytes([lo, hi])
    }

    // Registers

    #[inline(always)]
    pub fn set_al(&mut self, v: u8) {
        self.a = (self.a & 0xFF00) | u16::from(v);
    }

    #[inline(always)]
    pub fn nz8(&mut self, v: u8) {
        self.n = v & 0x80 != 0;
        self.z = v == 0;
    }

    #[inline(always)]
    pub fn nz16(&mut self, v: u16) {
        self.n = v & 0x8000 != 0;
        self.z = v == 0;
    }

    // Arithmetic

    pub fn adc8(&mut self, v: u8) {
        if self.dec {
            self.adc_bcd(v);
            return;
        }
        let a = self.a as u8;
        let cin = u8::from(self.c);
        let (s1, c1) = a.overflowing_add(v);
        let (sum, c2) = s1.overflowing_add(cin);
        let carry = c1 || c2;
        let bit6 = (a & 0x7F) + (v & 0x7F) + cin >= 0x80;
        self.v = bit6 != carry;
        self.c = carry;
        self.set_al(sum);
        self.nz8(sum);
    }

    pub fn adc16(&mut self, v: u16) {
        if self.dec {
            let [lo, hi] = v.to_le_bytes();
            self.adc_bcd(lo);
            self.a = self.a.swap_bytes();
            self.adc_bcd(hi);
            self.a = self.a.swap_bytes();
            self.z = self.a == 0;
            return;
        }
        let a = self.a;
        let cin = u16::from(self.c);
        let (s1, c1) = a.overflowing_add(v);
        let (sum, c2) = s1.overflowing_add(cin);
        let carry = c1 || c2;
        let bit14 = (a & 0x7FFF) + (v & 0x7FFF) + cin >= 0x8000;
        self.v = bit14 != carry;
        self.c = carry;
        self.a = sum;
        self.nz16(sum);
    }

    fn adc_bcd(&mut self, v: u8) {
        let original = self.a as u8;
        let cin = u8::from(self.c);
        let mut al = (original & 0x0F) + (v & 0x0F) + cin;
        if al >= 0x0A {
            al = 0x10 | ((al + 0x06) & 0x0F);
        }
        let mut a = u16::from(original & 0xF0) + u16::from(v & 0xF0) + u16::from(al);
        if a >= 0xA0 {
            a += 0x60;
        }
        let s = i16::from((original & 0xF0) as i8) + i16::from((v & 0xF0) as i8) + i16::from(al);
        let overflow = !(-128..128).contains(&s);
        let result = a as u8;
        self.set_al(result);
        self.z = result == 0;
        self.c = a >= 0x0100;
        self.n = result & 0x80 != 0;
        self.v = overflow;
    }

    pub fn sbc8(&mut self, v: u8) {
        if self.dec {
            self.sbc_bcd(v);
            return;
        }
        let a = self.a as u8;
        let borrow_in = u8::from(!self.c);
        let (d1, b1) = a.overflowing_sub(v);
        let (diff, b2) = d1.overflowing_sub(borrow_in);
        let borrow = b1 || b2;
        let bit6 = a & 0x7F < (v & 0x7F) + borrow_in;
        self.v = bit6 != borrow;
        self.c = !borrow;
        self.set_al(diff);
        self.nz8(diff);
    }

    pub fn sbc16(&mut self, v: u16) {
        if self.dec {
            let [lo, hi] = v.to_le_bytes();
            self.sbc_bcd(lo);
            self.a = self.a.swap_bytes();
            self.sbc_bcd(hi);
            self.a = self.a.swap_bytes();
            self.z = self.a == 0;
            return;
        }
        let a = self.a;
        let borrow_in = u16::from(!self.c);
        let (d1, b1) = a.overflowing_sub(v);
        let (diff, b2) = d1.overflowing_sub(borrow_in);
        let borrow = b1 || b2;
        let bit14 = a & 0x7FFF < (v & 0x7FFF) + borrow_in;
        self.v = bit14 != borrow;
        self.c = !borrow;
        self.a = diff;
        self.nz16(diff);
    }

    fn sbc_bcd(&mut self, v: u8) {
        let original = self.a as u8;
        let borrow_in = u8::from(!self.c);
        let mut al = u16::from(original & 0x0F)
            .wrapping_sub(u16::from(v & 0x0F))
            .wrapping_sub(u16::from(borrow_in));
        if al & 0x8000 != 0 {
            al = (al.wrapping_sub(0x06) & 0x0F).wrapping_sub(0x10);
        }
        let mut a = u16::from(original & 0xF0).wrapping_sub(u16::from(v & 0xF0)).wrapping_add(al);
        if a & 0x8000 != 0 {
            a = a.wrapping_sub(0x60);
        }
        let borrow = u16::from(original) < u16::from(v) + u16::from(borrow_in);
        let bit6 = original & 0x7F < (v & 0x7F) + borrow_in;
        let result = a as u8;
        self.set_al(result);
        self.z = result == 0;
        self.n = result & 0x80 != 0;
        self.c = !borrow;
        self.v = bit6 != borrow;
    }

    #[inline(always)]
    pub fn cmp8(&mut self, reg: u8, v: u8) {
        let (diff, borrow) = reg.overflowing_sub(v);
        self.c = !borrow;
        self.nz8(diff);
    }

    #[inline(always)]
    pub fn cmp16(&mut self, reg: u16, v: u16) {
        let (diff, borrow) = reg.overflowing_sub(v);
        self.c = !borrow;
        self.nz16(diff);
    }

    #[inline(always)]
    pub fn asl8(&mut self, v: u8) -> u8 {
        self.c = v & 0x80 != 0;
        let r = v << 1;
        self.nz8(r);
        r
    }

    #[inline(always)]
    pub fn asl16(&mut self, v: u16) -> u16 {
        self.c = v & 0x8000 != 0;
        let r = v << 1;
        self.nz16(r);
        r
    }

    #[inline(always)]
    pub fn lsr8(&mut self, v: u8) -> u8 {
        self.c = v & 1 != 0;
        let r = v >> 1;
        self.nz8(r);
        r
    }

    #[inline(always)]
    pub fn lsr16(&mut self, v: u16) -> u16 {
        self.c = v & 1 != 0;
        let r = v >> 1;
        self.nz16(r);
        r
    }

    #[inline(always)]
    pub fn rol8(&mut self, v: u8) -> u8 {
        let r = (v << 1) | u8::from(self.c);
        self.c = v & 0x80 != 0;
        self.nz8(r);
        r
    }

    #[inline(always)]
    pub fn rol16(&mut self, v: u16) -> u16 {
        let r = (v << 1) | u16::from(self.c);
        self.c = v & 0x8000 != 0;
        self.nz16(r);
        r
    }

    #[inline(always)]
    pub fn ror8(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (u8::from(self.c) << 7);
        self.c = v & 1 != 0;
        self.nz8(r);
        r
    }

    #[inline(always)]
    pub fn ror16(&mut self, v: u16) -> u16 {
        let r = (v >> 1) | (u16::from(self.c) << 15);
        self.c = v & 1 != 0;
        self.nz16(r);
        r
    }

    #[inline(always)]
    pub fn bit8(&mut self, v: u8) {
        self.n = v & 0x80 != 0;
        self.v = v & 0x40 != 0;
        self.z = (self.a as u8) & v == 0;
    }

    #[inline(always)]
    pub fn bit16(&mut self, v: u16) {
        self.n = v & 0x8000 != 0;
        self.v = v & 0x4000 != 0;
        self.z = self.a & v == 0;
    }

    /// `SEP`/`REP` on the flags other than M and X.
    #[inline(always)]
    pub fn set_flags(&mut self, bits: u8, on: bool) {
        if bits & 0x80 != 0 {
            self.n = on;
        }
        if bits & 0x40 != 0 {
            self.v = on;
        }
        if bits & 0x08 != 0 {
            self.dec = on;
        }
        if bits & 0x04 != 0 {
            self.irq = on;
        }
        if bits & 0x02 != 0 {
            self.z = on;
        }
        if bits & 0x01 != 0 {
            self.c = on;
        }
    }
}

/// A generated routine, as the table of the generated source lists it.
#[derive(Clone, Copy)]
pub struct GenEntry {
    /// The flags of P a `Verify` call compares (bit order NVMXDIZC): not those a poll of
    /// `$4210-$4212` sets from a read that lands a few master cycles apart from the original's.
    pub flags: u8,
    /// Physical key (24-bit address).
    pub key: u32,
    pub name: &'static str,
    pub func: fn(&mut Ctx<'_>),
    /// The cost bound of the first stretch for the entry state; `u64::MAX` declines the call.
    pub bound: fn(&Ctx<'_>) -> u64,
    /// Returns with RTL (else RTS).
    pub long: bool,
}

/// A generated function variant: its entry, widths and place in the generated `COV` table (one
/// flag per block, set when the block runs).
#[derive(Clone, Copy, Debug)]
pub struct Variant {
    /// Entry address (24-bit, bank bit 7 clear).
    pub entry: u32,
    /// The M/X bits of P (`0x20`, `0x10`) the variant runs with.
    pub mx: u8,
    /// Index of the first block in `COV`.
    pub cov_base: u32,
    /// Blocks of the variant.
    pub blocks: u32,
    /// Instructions of the variant.
    pub insns: u32,
    /// The generated function's name.
    pub name: &'static str,
}

/// A block of a function variant the original can arrive at (see [`MidEntry`]).
#[derive(Clone, Copy)]
pub struct MidVariant {
    /// The M/X bits of P the block runs with.
    pub mx: u8,
    pub func: fn(&mut Ctx<'_>, &mut R) -> u32,
    /// The block's number in the function.
    pub block: u32,
    /// The worst-case cost of the block.
    pub cost: u32,
}

/// An address inside generated code where a routine can start when the original arrives there
/// (after a hand-over), by the widths the CPU has.
#[derive(Clone, Copy)]
pub struct MidEntry {
    pub key: u32,
    pub long: bool,
    pub variants: &'static [MidVariant],
}

/// Runs the generated function from the block of the variant that fits the CPU's widths.
pub fn run_mid(ctx: &mut Ctx<'_>, e: &'static MidEntry) {
    let mut r = R::load(ctx);
    r.entry = e.key;
    let ret = match e.variants.iter().find(|v| v.mx == r.mx) {
        Some(v) => {
            r.start = v.block;
            (v.func)(ctx, &mut r)
        }
        None => r.bail(e.key, r.mx),
    };
    r.finish(ctx, ret, if e.long { 3 } else { 2 });
}

/// The cost bound of the block of the variant that fits the CPU's widths.
#[must_use]
pub fn mid_bound(ctx: &Ctx<'_>, e: &'static MidEntry) -> u64 {
    if ctx.pbr() != (e.key >> 16) as u8 {
        return u64::MAX;
    }
    let mx = (u8::from(ctx.m8()) << 5) | (u8::from(ctx.x8()) << 4);
    e.variants.iter().find(|v| v.mx == mx).map_or(u64::MAX, |v| u64::from(v.cost))
}

trait MaxEmpty {
    fn max_empty(self) -> &'static str;
}

impl MaxEmpty for &'static str {
    fn max_empty(self) -> &'static str {
        if self.is_empty() { "explicit" } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs() -> R {
        R {
            a: 0,
            x: 0,
            y: 0,
            s: 0x01FF,
            d: 0,
            dbr: 0,
            n: false,
            z: false,
            c: false,
            v: false,
            dec: false,
            irq: true,
            mx: 0x20,
            cy: 0,
            hist: 0,
            hn: 0,
            charged: 0,
            slack: 0,
            instrs: 0,
            resume: 0,
            depth: 0,
            nogate: false,
            stop: false,
            nopoll: false,
            entry: 0,
            why: "",
            start: 0,
            rl: false,
            low: 0x01FF,
            s0: 0x01FF,
            stop_after: u64::MAX,
            limit: u64::MAX,
            dropped: false,
        }
    }

    #[test]
    fn adc_sets_carry_and_overflow() {
        let mut r = regs();
        r.a = 0x1250;
        r.adc8(0x50);
        assert_eq!((r.a, r.c, r.v, r.n, r.z), (0x12A0, false, true, true, false));
        r.adc8(0x60);
        assert_eq!((r.a, r.c, r.v, r.z), (0x1200, true, false, true));
        r.c = true;
        r.a = 0xFFFF;
        r.adc16(0);
        assert_eq!((r.a, r.c, r.z), (0, true, true));
    }

    #[test]
    fn decimal_arithmetic_uses_bcd() {
        let mut r = regs();
        r.dec = true;
        r.a = 0x45;
        r.adc8(0x38);
        assert_eq!((r.a, r.c), (0x83, false));
        r.c = true;
        r.sbc8(0x19);
        assert_eq!((r.a, r.c), (0x64, true));
    }

    #[test]
    fn cycles_follow_the_region() {
        assert_eq!(spd(0x00_8000), 8);
        assert_eq!(spd(0x7E_2100), 8);
        assert_eq!(spd(0x00_2100), 6);
        assert_eq!(spd(0x00_4016), 12);
        assert_eq!(spd(0x80_4200), 6);
        assert!(is_io(0x00_4212) && !is_io(0x7E_4212) && !is_io(0x00_1000));
        assert_eq!(win(100), 148);
        assert_eq!(win(1300), 1300 + 2 * 40 + 8);
    }

    #[test]
    fn p_round_trips() {
        let mut r = regs();
        let widths = r.set_p(0xFF);
        assert_eq!(
            (widths, r.n, r.v, r.dec, r.irq, r.z, r.c),
            (0x30, true, true, true, true, true, true)
        );
        assert_eq!(r.p(widths), 0xFF);
        r.x = 0x1234;
        r.set_p(0x10);
        assert_eq!(r.x, 0x0034);
    }

    #[test]
    fn returns_resume_after_the_pulled_address() {
        let mut r = regs();
        assert_eq!(r.bail_ret(0x9A6C, false, 0x04), BAIL);
        assert_eq!(r.resume, 0x04_9A6D);
        assert_eq!(r.bail_ret(0x03_FFFF, true, 0x04), BAIL);
        assert_eq!(r.resume, 0x03_0000);
    }
}
