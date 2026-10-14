//! The coverage metric (spec §5.4): what share of the instructions a run executes now runs as
//! Rust?
//!
//! Instructions are 65816 opcode fetches. A machine counts those it executes itself
//! ([`Machine::opcode_fetches`]): the original code, which a routine in `Rust` mode no longer
//! runs, so its instructions are not among them. How many instructions a Rust call stands for is
//! measured by `Verify`: [`HookStats::mean_instructions`], the mean opcode fetches of the
//! original per compared call. A machine in `Rust` mode takes those means from a `Verify` run of
//! the same replay ([`Machine::set_mean_instructions`]) and its coverage is
//!
//! ```text
//! rust     = Σ (rust_runs × mean_instructions − callee_instructions)   over its routines
//! original = its opcode fetches
//! percent  = 100 × rust / (rust + original)
//! ```
//!
//! `rust + original` is what the original would have executed, so the percent is the share of
//! the run's instructions that were Rust.
//!
//! # What drives the figure
//!
//! The percent counts instructions, not game logic. Routines called from the idle wait between
//! frames dominate it: the two RNGs run from the idle loop (about 250 calls a frame on the
//! battle replay, 11 instructions each) and are nearly all of the Rust instructions there. The
//! percent grows with the time the game spends idling per frame, whatever the routines do, and
//! the idle loop's own instructions stay original until the whole frame task is Rust (spec
//! §5.3). A conservative second figure leaves chosen routines out of the Rust side
//! ([`Coverage::excluding`], `xtask coverage --exclude`).
//!
//! # What it estimates
//!
//! The mean is over the `Verify` calls, so a routine whose instruction count varies from call
//! to call (a loop of varying length) is counted at its mean. That is right only if its `Rust`
//! runs are a random sample of its calls, and they are not: the timing window sends the calls
//! near a timing event to the original, so a routine whose length depends on where in the frame
//! it is called is estimated off by the difference. Such a routine declares its instructions per
//! call ([`Ctx::set_instructions`](crate::Ctx::set_instructions), checked in `Verify`) and is
//! counted exactly. A routine's mean holds the
//! instructions of everything the original ran inside it, the calls to other routines included.
//! When its Rust run has original code run through `Ctx::call_original` (a routine that is
//! hooked or not), the machine fetches those instructions itself, or the callee's own runs
//! count them if it is in `Rust` mode too: the routine's runs are counted less
//! those ([`HookStats::callee_instructions`]), so nothing counts twice.

use crate::{HookStats, Machine};

/// Instructions that ran as original code and instructions that ran as Rust (estimated), over a
/// run; see the module documentation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Instructions the machine executed: opcode fetches since power-on.
    pub original_instructions: u64,
    /// Instructions the Rust runs stand for: `Σ rust_runs × mean_instructions` less what the
    /// runs had run elsewhere ([`HookStats::estimated_rust_instructions`]).
    pub rust_instructions_estimated: u64,
}

impl Coverage {
    /// The share of the run's instructions that ran as Rust, in percent:
    /// `100 × rust / (rust + original)`; 0 when there is no instruction at all.
    #[must_use]
    pub fn percent(&self) -> f64 {
        let rust = self.rust_instructions_estimated;
        let total = rust + self.original_instructions;
        if total == 0 { 0.0 } else { 100.0 * rust as f64 / total as f64 }
    }

    /// The coverage without `instructions` of the Rust side (the estimate of the routines to
    /// leave out, at most all of it): `(rust − excluded) / (rust − excluded + original)` is its
    /// [`percent`](Coverage::percent). The original instructions stay as they are. This is the
    /// conservative figure: it takes routines that dominate the run, such as those of the idle
    /// wait, out of the numerator and the denominator alike.
    #[must_use]
    pub fn excluding(&self, instructions: u64) -> Self {
        Self {
            original_instructions: self.original_instructions,
            rust_instructions_estimated: self
                .rust_instructions_estimated
                .saturating_sub(instructions),
        }
    }
}

/// The statistics of one frame: the differences of the machine's counters between the render
/// of the previous frame and the render of this one ([`Machine::last_frame_stats`]).
///
/// The counters are those of [`Machine::coverage`], so the sums over whole frames equal it at a
/// frame boundary. A Rust run is counted before its dispatch holds the CPU for the time it is
/// charged, which can render a frame: a run whose dispatch spans the render is in the frame it
/// started in, like the dispatch itself (`dispatches`, counted at the arrival), so the estimate
/// of a frame never dips below the previous one's.
/// After [`Machine::set_hook_table`] or [`Machine::set_mean_instructions`] the Rust estimate
/// starts again from the new figure (the frame they change is not charged for the change).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// The frame, as in [`Machine::frame_count`] before its render: the one whose joypad
    /// ([`Replay::input`](crate::Replay::input)) was held during it (0 = the first).
    pub frame: u64,
    /// Instructions the machine executed in the frame: opcode fetches.
    pub original_instructions: u64,
    /// Instructions the Rust runs of the frame stand for ([`Coverage::rust_instructions_estimated`]).
    pub rust_instructions_estimated: u64,
    /// Master cycles of the frame, as [`Machine::total_master_cycles`] counts them.
    pub master_cycles: u64,
    /// NMIs taken in the frame ([`Machine::nmi_count`]).
    pub nmis: u64,
    /// Arrivals at routine entries the machine dispatched (a sync point or a breakpoint is not
    /// one): Rust runs, Verify calls and the arrivals that fell back to the original, nested
    /// ones included.
    pub dispatches: u64,
}

impl FrameStats {
    /// The share of the frame's instructions that ran as Rust, in percent
    /// ([`Coverage::percent`]).
    #[must_use]
    pub fn coverage_percent(&self) -> f64 {
        Coverage {
            original_instructions: self.original_instructions,
            rust_instructions_estimated: self.rust_instructions_estimated,
        }
        .percent()
    }
}

impl HookStats {
    /// The instructions the `Rust` calls of this routine stand for: the runs that declared their
    /// instructions ([`Ctx::set_instructions`](crate::Ctx::set_instructions)) at the count they
    /// declared, the others at `mean_instructions` each, less
    /// [`HookStats::callee_instructions`], rounded and never below 0 (the means are 0 until a
    /// `Verify` run has measured them).
    #[must_use]
    pub fn estimated_rust_instructions(&self) -> u64 {
        let by_mean = self.rust_runs.saturating_sub(self.declared_runs) as f64;
        (by_mean * self.mean_instructions + self.declared_instructions - self.callee_instructions)
            .max(0.0)
            .round() as u64
    }
}

impl Machine {
    /// The coverage of the run so far: the instructions this machine executed (opcode fetches
    /// since power-on, [`Machine::opcode_fetches`]) and the sum over its routines of
    /// [`HookStats::estimated_rust_instructions`]. See the module documentation for the formula
    /// and what it estimates.
    #[must_use]
    pub fn coverage(&self) -> Coverage {
        Coverage {
            original_instructions: self.opcode_fetches(),
            rust_instructions_estimated: self.rust_instructions_estimated(),
        }
    }
}
