//! Binary to packed BCD by repeated subtraction, for HUD and status numbers.
//!
//! The routine works on the direct page: `$00–$02` the binary value (left as the remainder),
//! `$03` tens|ones, `$04` thousands|hundreds, `$05` 100k|10k, `$06` millions (a binary count),
//! `$07–$08` scratch. These direct-page bytes stay implicit: `re/symbols.toml` names no variable
//! for them. Six stages each subtract a power of ten while they can; a stage that adds `$10` to
//! its digit byte also stops when that byte wraps to 0, and stages 4–6 only look at the low
//! bytes of the value. Both quirks only show on inputs the 24-bit entry never produces (the
//! suffix entries at stages 4, 5 and 6 can reach them) and are replicated, not fixed.

use ssd_oracle::Ctx;

/// Direct-page bytes the routine works on (`$00–$08`).
const DP_BYTES: usize = 9;
/// First scratch byte (`$07`): the low bytes of each trial subtraction.
const SCRATCH: usize = 7;
/// `$03`: tens|ones; the ones are merged in at the end.
const TENS_ONES: usize = 3;

/// How a stage counts a subtraction in its digit byte.
#[derive(Debug, Clone, Copy)]
enum Count {
    /// Add 1, C/V untouched; the loop ends if the byte wraps to 0 (never in practice: 256
    /// steps).
    Inc,
    /// Add `$10` (the high digit), C/V from that addition; the loop ends when the byte wraps
    /// to 0 (after 16 steps).
    AddTen,
}

/// One subtraction loop.
#[derive(Debug, Clone, Copy)]
struct Stage {
    /// Bytes of the value it subtracts from (`$00` up), 1 to 3.
    width: usize,
    divisor: u32,
    /// Direct-page byte counting the subtractions.
    digit: usize,
    count: Count,
}

/// CPU-busy master cycles of a call from the 24-bit entry that subtracts nothing, from the hit
/// to the return: zeroing the digits, one trial subtraction per stage (which borrows), and
/// merging the ones.
const COST_BASE: u64 = 1202;
/// What each subtraction that goes through adds, by stage: the trial, storing the result back,
/// counting it in the digit byte and looping. The 3-byte stages cost most (stage 2, counting
/// with an addition, more than stages 1 and 3), the 1-byte tens stage least. Costs measured in
/// `Verify`; the worst case, `cycles` in `re/symbols.toml`, is checked by the tests.
const COST_PER_SUBTRACTION: [u64; 6] = [378, 418, 378, 306, 266, 194];
/// The cost of a call from the 24-bit entry that made `subtractions` (per stage): a closed
/// formula of the loop counts. From that entry no digit byte wraps (at most 16 millions and 9
/// of each other digit), so every stage ends on a borrowing trial, which [`COST_BASE`] counts.
pub(crate) fn cost(subtractions: &[u32; 6]) -> u64 {
    let looped: u64 = COST_PER_SUBTRACTION
        .iter()
        .zip(subtractions)
        .map(|(&per_subtraction, &count)| per_subtraction * u64::from(count))
        .sum();
    COST_BASE + looped
}

/// Instructions of a call from the 24-bit entry that subtracts nothing: `SEP`, four `STZ`, one
/// borrowing trial per stage (10, 10, 10, 7, 7 and 4 instructions), and the merge of the ones
/// (`LDA`, `ORA`, `STA`, `RTL`).
const INSTRUCTIONS_BASE: u64 = 57;
/// What each subtraction that goes through adds, by stage: the trial that does not borrow plus
/// storing the results and counting the digit. Like the cost, declared per call
/// (`Ctx::set_instructions`): the few calls are a sample the timing window picks.
const INSTRUCTIONS_PER_SUBTRACTION: [u64; 6] = [17, 20, 17, 15, 12, 10];
/// The instructions of a call from the 24-bit entry that made `subtractions` (per stage).
pub(crate) fn instructions(subtractions: &[u32; 6]) -> u64 {
    let looped: u64 = INSTRUCTIONS_PER_SUBTRACTION
        .iter()
        .zip(subtractions)
        .map(|(&per_subtraction, &count)| per_subtraction * u64::from(count))
        .sum();
    INSTRUCTIONS_BASE + looped
}

/// The six stages; stages 4, 5 and 6 (indices 3–5) are the suffix entries.
const STAGES: [Stage; 6] = [
    Stage { width: 3, divisor: 1_000_000, digit: 6, count: Count::Inc },
    Stage { width: 3, divisor: 100_000, digit: 5, count: Count::AddTen },
    Stage { width: 3, divisor: 10_000, digit: 5, count: Count::Inc },
    Stage { width: 2, divisor: 1_000, digit: 4, count: Count::AddTen },
    Stage { width: 2, divisor: 100, digit: 4, count: Count::Inc },
    Stage { width: 1, divisor: 10, digit: 3, count: Count::AddTen },
];

/// The routine's state: the direct-page bytes, and C/V as the last subtraction or digit
/// addition left them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bcd {
    pub dp: [u8; DP_BYTES],
    pub c: bool,
    pub v: bool,
}

impl Bcd {
    /// The 24-bit entry: zeroes the digits `$03–$06` and runs every stage. Returns A (`$03`)
    /// and the subtractions each stage made.
    pub(crate) fn convert(&mut self) -> (u8, [u32; 6]) {
        self.dp[TENS_ONES..SCRATCH].fill(0);
        self.run_from(0)
    }

    /// Runs the stages from `first` (0–5) on, then merges the ones left in `$00` into `$03`
    /// (bitwise or). Returns A, the merged byte, and the subtractions each stage made (0 for
    /// the stages before `first`).
    pub(crate) fn run_from(&mut self, first: usize) -> (u8, [u32; 6]) {
        let mut subtractions = [0; 6];
        for (count, stage) in subtractions.iter_mut().zip(&STAGES).skip(first) {
            *count = self.run(stage);
        }
        let a = self.dp[TENS_ONES] | self.dp[0];
        self.dp[TENS_ONES] = a;
        (a, subtractions)
    }

    /// One stage: a trial subtraction of the divisor, byte by byte from the low byte with the
    /// borrow chained; the low results go to the scratch bytes, the top one is held. A final
    /// borrow ends the stage. Otherwise the results are stored back, the digit byte counts the
    /// subtraction, and the loop goes on unless that byte wrapped to 0. Returns the
    /// subtractions that went through.
    fn run(&mut self, stage: &Stage) -> u32 {
        let top = stage.width - 1;
        let mut subtractions = 0;
        loop {
            let mut carry = true;
            let mut top_result = 0;
            for (i, divisor_byte) in stage.divisor.to_le_bytes()[..stage.width].iter().enumerate() {
                let (result, c, v) = sbc(self.dp[i], *divisor_byte, carry);
                carry = c;
                if i < top {
                    self.dp[SCRATCH + i] = result;
                } else {
                    top_result = result;
                    // The top byte's subtraction sets C and V last.
                    self.v = v;
                }
            }
            self.c = carry;
            if !carry {
                return subtractions;
            }
            subtractions += 1;
            self.dp[top] = top_result;
            self.dp.copy_within(SCRATCH..SCRATCH + top, 0);
            let digit = match stage.count {
                Count::Inc => self.dp[stage.digit].wrapping_add(1),
                Count::AddTen => {
                    let (result, c, v) = adc(self.dp[stage.digit], 0x10, false);
                    (self.c, self.v) = (c, v);
                    result
                }
            };
            self.dp[stage.digit] = digit;
            if digit == 0 {
                return subtractions;
            }
        }
    }
}

/// 8-bit binary subtraction with borrow, `a - b - !carry`: the addition of `!b`. Returns the
/// result, C (no borrow) and V.
fn sbc(a: u8, b: u8, carry: bool) -> (u8, bool, bool) {
    adc(a, !b, carry)
}

/// 8-bit binary addition with carry, `a + b + carry`. Returns the result, C and V.
fn adc(a: u8, b: u8, carry: bool) -> (u8, bool, bool) {
    let sum = u16::from(a) + u16::from(b) + u16::from(carry);
    let result = sum as u8;
    let v = (a ^ result) & (b ^ result) & 0x80 != 0;
    (result, sum > 0xFF, v)
}

/// `bin24_to_packed_bcd`: [`Bcd::convert`] on the direct page. A low = `$03` (B untouched), N/Z
/// from that byte, C/V from the last subtraction or digit addition; X and Y untouched. Costs
/// [`cost`] of its subtractions.
pub(crate) fn bin24_to_packed_bcd(ctx: &mut Ctx<'_>) {
    let mut bcd = Bcd { dp: [0; DP_BYTES], c: ctx.c(), v: ctx.v() };
    for (offset, byte) in (0_u16..).zip(&mut bcd.dp) {
        *byte = ctx.read_dp8(offset);
    }
    let (a, subtractions) = bcd.convert();
    for (offset, &byte) in (0_u16..).zip(&bcd.dp) {
        ctx.write_dp8(offset, byte);
    }
    ctx.set_al(a);
    ctx.set_nz8(a);
    ctx.set_c(bcd.c);
    ctx.set_v(bcd.v);
    ctx.set_cost(cost(&subtractions));
    ctx.set_instructions(instructions(&subtractions));
}

#[cfg(test)]
mod tests {
    use super::{Bcd, cost};
    use crate::routines::tests::max_cost;

    /// The worst case of the 24-bit entry, 15 999 990 (15 millions, then 9 of each lower digit).
    const MAX_COST: u64 = 20_930;

    /// The cost of a call from the 24-bit entry for `value`, through the model's loop counts.
    fn cost_of(value: u32) -> u64 {
        cost(&input(value).convert().1)
    }

    #[test]
    fn the_cost_follows_the_loop_counts() {
        // Measured on the original, the hit to the return, with the value poked in.
        let measured = [
            (0, 1202),
            (9, 1202),
            (10, 1396),
            (99, 2948),
            (100, 1468),
            (4_321, 3612),
            (123_456, 5328),
            (909_090, 9464),
            (999_999, 15_260),
            (1_000_000, 1580),
            (5_555_555, 10_902),
            (15_999_999, 20_930),
            (16_000_000, 7250),
            (16_777_215, 15_690),
        ];
        for (value, cycles) in measured {
            assert_eq!(cost_of(value), cycles, "{value}");
        }
        let (_, subtractions) = input(16_777_215).convert();
        assert_eq!(subtractions, [16, 7, 7, 7, 2, 1], "the digits, millions counted in binary");
    }

    #[test]
    fn the_symbols_cost_is_the_worst_case() {
        // The cost depends on the digits only: every value's digits, millions up to 16.
        let mut worst = (0, 0);
        for millions in 0..=16 {
            for lower in (0..1_000_000).step_by(10) {
                let value = millions * 1_000_000 + lower;
                if value > 0xFF_FFFF {
                    break;
                }
                let digits = [5, 4, 3, 2, 1].map(|power| (lower / 10_u32.pow(power)) % 10);
                let [d5, d4, d3, d2, d1] = digits;
                let cycles = cost(&[millions, d5, d4, d3, d2, d1]);
                if cycles > worst.0 {
                    worst = (cycles, value);
                }
            }
        }
        assert_eq!(worst, (MAX_COST, 15_999_990));
        assert_eq!(cost_of(15_999_990), MAX_COST);
        assert_eq!(max_cost("bin24_to_packed_bcd"), MAX_COST);
    }

    /// The state for `value` in `$00–$02`, with digits and scratch holding junk.
    fn input(value: u32) -> Bcd {
        let [low, mid, high, _] = value.to_le_bytes();
        Bcd { dp: [low, mid, high, 0xAA, 0xAA, 0xAA, 0xAA, 0x55, 0x66], c: false, v: true }
    }

    /// Expected results, from an instruction-level model of the routine: the direct page after
    /// the call and A, N, V, Z, C.
    type Expected = ([u8; 9], u8, [u8; 4]);

    fn check(mut bcd: Bcd, run: impl FnOnce(&mut Bcd) -> u8, expected: Expected, what: &str) {
        let (dp, acc, [n, v, z, c]) = expected;
        let flags = |n: u8, z: u8| (n == 1, z == 1);
        assert_eq!(run(&mut bcd), acc, "{what}: A");
        assert_eq!(bcd.dp, dp, "{what}: direct page");
        // N/Z are those of A (the merged byte).
        assert_eq!((acc & 0x80 != 0, acc == 0), flags(n, z), "{what}: expected N/Z");
        assert_eq!((bcd.v, bcd.c), flags(v, c), "{what}: V/C");
    }

    #[test]
    fn converts_24_bit_values() {
        let cases: [(u32, Expected); 10] = [
            (0, ([0x00, 0, 0, 0x00, 0x00, 0x00, 0x00, 0x9C, 0xD8], 0x00, [0, 0, 1, 0])),
            (9, ([0x09, 0, 0, 0x09, 0x00, 0x00, 0x00, 0xA5, 0xD8], 0x09, [0, 0, 0, 0])),
            (10, ([0x00, 0, 0, 0x10, 0x00, 0x00, 0x00, 0xA6, 0xD8], 0x10, [0, 0, 0, 0])),
            (99, ([0x09, 0, 0, 0x99, 0x00, 0x00, 0x00, 0xFF, 0xD9], 0x99, [1, 0, 0, 0])),
            (100, ([0x00, 0, 0, 0x00, 0x01, 0x00, 0x00, 0x9C, 0xD9], 0x00, [0, 0, 1, 0])),
            (4_321, ([0x01, 0, 0, 0x21, 0x43, 0x00, 0x00, 0xB1, 0xE9], 0x21, [0, 0, 0, 0])),
            (123_456, ([0x06, 0, 0, 0x56, 0x34, 0x12, 0x00, 0xD4, 0xE6], 0x56, [0, 0, 0, 0])),
            (999_999, ([0x09, 0, 0, 0x99, 0x99, 0x99, 0x00, 0xFF, 0xFF], 0x99, [1, 0, 0, 0])),
            (1_000_000, ([0x00, 0, 0, 0x00, 0x00, 0x00, 0x01, 0x9C, 0xD8], 0x00, [0, 0, 1, 0])),
            // The millions are a binary count: 16 → $10.
            (16_777_215, ([0x05, 0, 0, 0x15, 0x72, 0x77, 0x10, 0xAB, 0xF5], 0x15, [0, 0, 0, 0])),
        ];
        for (value, expected) in cases {
            check(input(value), |bcd| bcd.convert().0, expected, &format!("{value}"));
        }
    }

    #[test]
    fn a_digit_byte_that_wraps_ends_its_stage() {
        // The tens stage with $00 = 200: after 16 subtractions $03 wraps to 0, leaving 40 in
        // $00, merged into $03 as garbage; C = 1 from the wrapping digit addition.
        let tens =
            |value: u8| Bcd { dp: [value, 0, 0, 0, 0, 0, 0, 0x55, 0x66], c: false, v: false };
        let expected = ([0x28, 0, 0, 0x28, 0, 0, 0, 0x55, 0x66], 0x28, [0, 0, 0, 1]);
        check(tens(200), |bcd| bcd.run_from(5).0, expected, "tens 200");
        let expected = ([0x5F, 0, 0, 0x5F, 0, 0, 0, 0x55, 0x66], 0x5F, [0, 0, 0, 1]);
        check(tens(255), |bcd| bcd.run_from(5).0, expected, "tens 255");

        // The thousands stage with 20 000: $04 wraps after 16 thousands, then the hundreds
        // stage counts the 4 000 left as 40 hundreds ($04 = $28).
        let thousands = Bcd { dp: [0x20, 0x4E, 0, 0, 0, 0, 0, 0x55, 0x66], c: false, v: false };
        let expected = ([0, 0, 0, 0, 0x28, 0, 0, 0x9C, 0x66], 0x00, [0, 0, 1, 0]);
        check(thousands, |bcd| bcd.run_from(3).0, expected, "thousands 20000");

        // The hundreds stage counts by 1: 6 000 is 60 hundreds ($3C), no wrap.
        let hundreds = Bcd { dp: [0x70, 0x17, 0, 0, 0, 0, 0, 0x55, 0x66], c: false, v: false };
        let expected = ([0, 0, 0, 0, 0x3C, 0, 0, 0x9C, 0x66], 0x00, [0, 0, 1, 0]);
        check(hundreds, |bcd| bcd.run_from(4).0, expected, "hundreds 6000");
    }

    #[test]
    fn the_suffix_stages_ignore_the_high_bytes() {
        // From the tens stage, $01 and $02 are neither read nor written.
        let mut bcd = Bcd { dp: [42, 0x12, 0x34, 0, 0, 0, 0, 0x55, 0x66], c: false, v: false };
        assert_eq!(bcd.run_from(5).0, 0x42);
        assert_eq!(bcd.dp, [2, 0x12, 0x34, 0x42, 0, 0, 0, 0x55, 0x66]);
    }
}
