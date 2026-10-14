//! The two random number generators: identical 16-bit linear congruential generators
//! `s = s * 5 + $11` on two seeds in the direct page. The idle loop calls them until the NMI,
//! so their cost decides the sequence (`docs/re/m1-candidates.md`, global caveat).

use crate::data;
use ssd_oracle::Ctx;

/// Direct-page offset of `rng_seed_a` (`re/symbols.toml`).
const SEED_A: u16 = (data::RNG_SEED_A & 0xFFFF) as u16;
/// Direct-page offset of `rng_seed_b`.
const SEED_B: u16 = (data::RNG_SEED_B & 0xFFFF) as u16;
/// CPU-busy master cycles of a call of either generator, from the hit to the return: one path
/// (measured in `Verify`).
pub(crate) const COST: u64 = 264;

/// One step of the generator and the flags its final 16-bit addition of `$11` leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    pub seed: u16,
    pub n: bool,
    pub v: bool,
    pub z: bool,
    pub c: bool,
}

/// `s * 5 + $11` in 16 bits, as two additions: `4s + s` (mod `$10000`, its carry dropped),
/// then that sum plus `$11` with no carry in. N/V/Z/C are those of the second addition.
pub(crate) fn step(seed: u16) -> Step {
    let sum = (seed << 2).wrapping_add(seed);
    let (next, c) = sum.overflowing_add(0x0011);
    // Overflow: both addends have the same sign and the result's differs.
    let v = (sum ^ next) & (0x0011 ^ next) & 0x8000 != 0;
    Step { seed: next, n: next & 0x8000 != 0, v, z: next == 0, c }
}

/// `rng_next_a`: advances `rng_seed_a`; A (all 16 bits) = the new seed, N/V/Z/C from the final
/// addition. The routine runs with a 16-bit accumulator and exits with M8, its entry width.
pub(crate) fn rng_next_a(ctx: &mut Ctx<'_>) {
    next(ctx, SEED_A);
}

/// `rng_next_b`: [`rng_next_a`] on `rng_seed_b`.
pub(crate) fn rng_next_b(ctx: &mut Ctx<'_>) {
    next(ctx, SEED_B);
}

fn next(ctx: &mut Ctx<'_>, seed: u16) {
    let step = step(ctx.read_dp16(seed));
    ctx.write_dp16(seed, step.seed);
    ctx.set_a(step.seed);
    ctx.set_nz16(step.seed);
    ctx.set_v(step.v);
    ctx.set_c(step.c);
    ctx.set_cost(COST);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_constant() {
        assert_eq!(max_cost("rng_next_a"), COST);
        assert_eq!(max_cost("rng_next_b"), COST);
    }

    /// `(seed, N, V, Z, C)` after each step, computed with an instruction-level model of the
    /// routine's 16-bit arithmetic, flag by flag).
    type Expected = (u16, u8, u8, u8, u8);

    fn check(mut seed: u16, expected: &[Expected]) {
        for (i, &(next, n, v, z, c)) in expected.iter().enumerate() {
            let step = step(seed);
            let flags = (step.n, step.v, step.z, step.c);
            assert_eq!(step.seed, next, "step {i} from {seed:04X}");
            assert_eq!(flags, (n == 1, v == 1, z == 1, c == 1), "NVZC of step {i} from {seed:04X}");
            seed = next;
        }
    }

    #[test]
    fn sequence_from_seed_0000() {
        check(
            0x0000,
            &[
                (0x0011, 0, 0, 0, 0),
                (0x0066, 0, 0, 0, 0),
                (0x020F, 0, 0, 0, 0),
                (0x0A5C, 0, 0, 0, 0),
                (0x33DD, 0, 0, 0, 0),
                (0x0362, 0, 0, 0, 0), // 5 * $33DD carries out of the first addition: discarded
                (0x10FB, 0, 0, 0, 0),
                (0x54F8, 0, 0, 0, 0),
                (0xA8E9, 1, 0, 0, 0),
                (0x4C9E, 0, 0, 0, 0),
                (0x7F27, 0, 0, 0, 0),
                (0x7BD4, 0, 0, 0, 0),
            ],
        );
    }

    #[test]
    fn sequence_from_seed_ffff() {
        check(
            0xFFFF,
            &[
                (0x000C, 0, 0, 0, 1), // $FFFB + $11 carries out
                (0x004D, 0, 0, 0, 0),
                (0x0192, 0, 0, 0, 0),
                (0x07EB, 0, 0, 0, 0),
                (0x27A8, 0, 0, 0, 0),
                (0xC659, 1, 0, 0, 0),
                (0xDFCE, 1, 0, 0, 0),
                (0x5F17, 0, 0, 0, 0),
                (0xDB84, 1, 0, 0, 0),
                (0x49A5, 0, 0, 0, 0),
                (0x704A, 0, 0, 0, 0),
                (0x3183, 0, 0, 0, 0),
            ],
        );
    }

    #[test]
    fn overflow_and_zero_come_from_the_final_addition() {
        // 5 * $B330 = $7FF0 (mod $10000): $7FF0 + $11 overflows into the sign bit.
        check(0xB330, &[(0x8001, 1, 1, 0, 0)]);
        // 5 * $6663 = $FFEF: + $11 wraps to zero with a carry.
        check(0x6663, &[(0x0000, 0, 0, 1, 1)]);
    }
}
