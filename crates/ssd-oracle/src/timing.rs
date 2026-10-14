//! The timing-event window of `Rust` mode (M1b): a Rust routine runs atomically at its entry and
//! is then charged its cost with the CPU held, so it cannot reproduce a timing event that would
//! have landed inside the original (an NMI or IRQ taken mid-routine, HDMA stealing cycles at a
//! phase that depends on the CPU's cycles). It runs only when no such event can come before the
//! original would have returned; otherwise the original runs.
//!
//! A routine that calls original code (`Ctx::call_original`) is atomic only in its Rust parts:
//! the code it calls is emulated, events in it included. The window still covers the whole call,
//! since the parts after a call come after a time the entry cannot know: the entry's `cycles` is
//! the maximum including the code it calls, and a call that takes longer counts in
//! `cost_over_bound`.
//!
//! An IRQ counts as a timing event only if the CPU can take it: a pending or due IRQ is ignored
//! while the I flag is set ([`Machine::master_cycles_until_timing_event`]). The window is
//! therefore clear for a routine that runs with I set, even when an IRQ is due inside its cost,
//! and that is right only while the routine leaves I set: a routine whose original clears I
//! (`CLI`, `REP #$04`, or a `PLP` or `RTI` pulling a P with I clear) would take that IRQ right
//! after, which the window of the entry did not see. Such a routine needs its own window check,
//! for the part after the instruction that clears I.
//!
//! The window assumes that routines start no GPDMA themselves: only a transfer requested before
//! the call is accounted for ([`window_is_clear`]). A future routine that writes MDMAEN has to
//! account for its transfer.

use crate::Machine;

/// Master cycles of one memory refresh, which pauses the CPU once per scanline.
const REFRESH_MCLK: u64 = 40;
/// The shortest scanline.
const MIN_LINE_MCLK: u64 = 1360;
/// Kept free after the call.
const MARGIN_MCLK: u64 = 8;

/// The master cycles a call that costs at most `max_cost` CPU-busy master cycles can span: the
/// cost, plus the memory refreshes that can fall inside it (one per scanline: a span of `t`
/// master cycles holds at most `t / 1360 + 1`, and the refreshes lengthen the span), plus a
/// margin. A call shorter than a scanline spans at most `max_cost + 48`.
pub(crate) fn window(max_cost: u64) -> u64 {
    // Saturating: a bound function declines a call it does not handle with a huge cost
    // (`u64::MAX`), which no event is ever beyond.
    let refreshes = max_cost.saturating_add(MIN_LINE_MCLK) / (MIN_LINE_MCLK - REFRESH_MCLK);
    max_cost.saturating_add(refreshes * REFRESH_MCLK).saturating_add(MARGIN_MCLK)
}

/// Whether a call costing at most `max_cost` can run in Rust now: no timing event (see
/// [`Machine::master_cycles_until_timing_event`]) within its [`window`], and no GPDMA requested
/// right before it (the transfer's phase depends on the CPU cycle it starts after, which the
/// held CPU does not reproduce).
pub(crate) fn window_is_clear(machine: &Machine, max_cost: u64) -> bool {
    call_fits_window(
        machine.master_cycles_until_timing_event(),
        machine.gpdma_requested(),
        max_cost,
    )
}

/// [`window_is_clear`] on its two readings: `until_event` ([`Machine::master_cycles_until_timing_event`]) and
/// whether a GPDMA was requested right before the call. Public for the tools that measure whether
/// a call of a known cost would have run in `Rust` mode (`HookStats::window_fit`, `xtask probe`).
#[must_use]
pub fn call_fits_window(until_event: u64, gpdma_requested: bool, cost: u64) -> bool {
    !gpdma_requested && until_event > window(cost)
}

#[cfg(test)]
mod tests {
    use super::{call_fits_window, window};

    #[test]
    fn a_call_shorter_than_a_scanline_spans_its_cost_plus_48() {
        for cost in [0, 128, 264, 1278] {
            assert_eq!(window(cost), cost + 48, "{cost}");
        }
    }

    #[test]
    fn a_call_fits_when_the_next_event_is_beyond_its_window_and_no_gpdma_is_requested() {
        let cost = 1000;
        assert!(call_fits_window(window(cost) + 1, false, cost));
        assert!(!call_fits_window(window(cost), false, cost), "the event at the window's end");
        assert!(!call_fits_window(0, false, cost), "an event is pending");
        assert!(call_fits_window(u64::MAX, false, 5_000_000), "no event can come");
        assert!(!call_fits_window(u64::MAX, true, cost), "a transfer was requested");
        // The window grows with the cost: a longer call needs a later event.
        assert!(call_fits_window(2000, false, 1000));
        assert!(!call_fits_window(2000, false, 2000));
    }

    #[test]
    fn a_huge_cost_never_fits_and_does_not_overflow() {
        assert_eq!(window(u64::MAX), u64::MAX);
        assert!(!call_fits_window(u64::MAX, false, u64::MAX));
    }

    #[test]
    fn longer_calls_add_a_refresh_per_scanline_they_can_span() {
        // A call with r refreshes spans cost + 40r master cycles, which hold at most
        // ceil((cost + 40r) / 1360) refresh points: r refreshes need 1320r < cost + 1360.
        for cost in [1280_u64, 1500, 2640, 5000, 10_000, 400_000] {
            let refreshes = (window(cost) - cost - 8) / 40;
            assert_eq!(window(cost), cost + 40 * refreshes + 8, "{cost}");
            assert!(1320 * (refreshes + 1) >= cost + 1360, "{cost}: one more cannot happen");
        }
        assert_eq!(window(2640), 2640 + 3 * 40 + 8);
    }
}
