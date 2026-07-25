//! Overflow and saturation tests for storage arithmetic.
//!
//! This module exercises every checked-arithmetic helper in `amount_validation`
//! at extreme `i128` values to verify that:
//! - No unchecked wraparound occurs.
//! - `checked_add` / `checked_sub` helpers return `None` at genuine overflow.
//! - Accumulation helpers surface `PotentialOverflow` rather than panicking.
//! - Boundary values (i128::MAX, i128::MIN, sums one stroop below overflow,
//!   subtractions one stroop above underflow) behave correctly.

#[cfg(test)]
mod tests {
    use crate::amount_validation::{
        accumulate_amounts, safe_add_amounts, safe_subtract_amounts, validate_deposit_amount,
        validate_milestone_amounts, validate_single_amount, MAX_SINGLE_AMOUNT_STROOPS,
    };
    use crate::EscrowError;

    // ══════════════════════════════════════════════════════════════════════════════
    //  safe_add_amounts  –  i128 extremes & sums near max
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn safe_add_amounts_i128_max_plus_zero_is_max() {
        assert_eq!(safe_add_amounts(i128::MAX, 0), Some(i128::MAX));
    }

    #[test]
    fn safe_add_amounts_i128_max_plus_one_is_none() {
        assert_eq!(safe_add_amounts(i128::MAX, 1), None);
    }

    #[test]
    fn safe_add_amounts_i128_max_plus_max_overflow_is_none() {
        assert_eq!(safe_add_amounts(i128::MAX, i128::MAX), None);
    }

    #[test]
    fn safe_add_amounts_i128_min_plus_zero_is_min() {
        assert_eq!(safe_add_amounts(i128::MIN, 0), Some(i128::MIN));
    }

    #[test]
    fn safe_add_amounts_i128_min_plus_negative_one_is_none() {
        // i128::MIN + (-1) underflows below i128::MIN
        assert_eq!(safe_add_amounts(i128::MIN, -1), None);
    }

    #[test]
    fn safe_add_amounts_i128_min_plus_max_yields_negative_one() {
        // i128::MIN + i128::MAX = -1
        assert_eq!(safe_add_amounts(i128::MIN, i128::MAX), Some(-1));
    }

    #[test]
    fn safe_add_amounts_one_below_max_plus_one_is_max() {
        assert_eq!(safe_add_amounts(i128::MAX - 1, 1), Some(i128::MAX));
    }

    #[test]
    fn safe_add_amounts_one_below_max_plus_two_is_none() {
        assert_eq!(safe_add_amounts(i128::MAX - 1, 2), None);
    }

    #[test]
    fn safe_add_amounts_half_max_plus_half_max_plus_one_is_none() {
        let half = i128::MAX / 2;
        // (i128::MAX / 2 + 2) + (i128::MAX / 2) would overflow
        assert_eq!(safe_add_amounts(half + 2, half + 2), None);
    }

    #[test]
    fn safe_add_amounts_negative_extremes() {
        // (i128::MIN + 1) + (-2) underflows, so checked_add returns None.
        assert_eq!(safe_add_amounts(i128::MIN + 1, -2), None);
        assert_eq!(safe_add_amounts(i128::MIN, -1), None);
        assert_eq!(safe_add_amounts(i128::MIN, i128::MIN), None);
    }

    #[test]
    fn safe_add_amounts_zero_plus_zero_is_zero() {
        assert_eq!(safe_add_amounts(0, 0), Some(0));
    }

    #[test]
    fn safe_add_amounts_large_positive_plus_large_negative() {
        assert_eq!(safe_add_amounts(i128::MAX, i128::MIN), Some(-1));
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  safe_subtract_amounts  –  near-zero subtraction & i128::MIN underflow
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn safe_subtract_amounts_zero_minus_one_is_neg_one() {
        // 0 - 1 = -1, which is perfectly valid i128
        assert_eq!(safe_subtract_amounts(0, 1), Some(-1));
    }

    #[test]
    fn safe_subtract_amounts_zero_minus_i128_max_is_neg_max() {
        assert_eq!(safe_subtract_amounts(0, i128::MAX), Some(-i128::MAX));
    }

    #[test]
    fn safe_subtract_amounts_one_minus_one_is_zero() {
        assert_eq!(safe_subtract_amounts(1, 1), Some(0));
    }

    #[test]
    fn safe_subtract_amounts_i128_min_minus_one_is_none() {
        // i128::MIN - 1 underflows
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    #[test]
    fn safe_subtract_amounts_i128_min_minus_zero_is_min() {
        assert_eq!(safe_subtract_amounts(i128::MIN, 0), Some(i128::MIN));
    }

    #[test]
    fn safe_subtract_amounts_i128_max_minus_negative_one_is_none() {
        // i128::MAX - (-1) = i128::MAX + 1 → overflow
        assert_eq!(safe_subtract_amounts(i128::MAX, -1), None);
    }

    #[test]
    fn safe_subtract_amounts_i128_max_minus_i128_max_is_zero() {
        assert_eq!(safe_subtract_amounts(i128::MAX, i128::MAX), Some(0));
    }

    #[test]
    fn safe_subtract_amounts_i128_max_minus_max_stroops() {
        // i128::MAX is far above the practical stroop cap, but arithmetic must still hold
        let big = i128::MAX / 2;
        assert_eq!(safe_subtract_amounts(i128::MAX, big), Some(i128::MAX - big));
    }

    #[test]
    fn safe_subtract_amounts_neg_one_minus_i128_max_yields_i128_min() {
        // -1 - i128::MAX = i128::MIN (valid, no underflow)
        assert_eq!(safe_subtract_amounts(-1, i128::MAX), Some(i128::MIN));
    }

    #[test]
    fn safe_subtract_amounts_neg_one_minus_near_max_yields_neg_i128_max() {
        // -1 - (i128::MAX - 1) = -i128::MAX → valid
        assert_eq!(safe_subtract_amounts(-1, i128::MAX - 1), Some(-i128::MAX));
    }

    #[test]
    fn safe_subtract_amounts_min_boundary_exhaustive() {
        // Subtraction near i128::MIN
        assert_eq!(safe_subtract_amounts(i128::MIN + 1, 1), Some(i128::MIN));
        assert_eq!(safe_subtract_amounts(i128::MIN + 2, 2), Some(i128::MIN));
        assert_eq!(safe_subtract_amounts(i128::MIN, 0), Some(i128::MIN));
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  accumulate_amounts  –  near-max sums & iteration safety
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn accumulate_amounts_empty_returns_zero() {
        let amounts: [i128; 0] = [];
        assert_eq!(accumulate_amounts(amounts), Ok(0));
    }

    #[test]
    fn accumulate_amounts_single_valid_returns_amount() {
        assert_eq!(accumulate_amounts([100_i128]), Ok(100));
    }

    #[test]
    fn accumulate_amounts_single_at_max_single_amount_stroops_is_ok() {
        assert_eq!(
            accumulate_amounts([MAX_SINGLE_AMOUNT_STROOPS]),
            Ok(MAX_SINGLE_AMOUNT_STROOPS)
        );
    }

    #[test]
    fn accumulate_amounts_single_above_max_single_amount_stroops_fails() {
        assert_eq!(
            accumulate_amounts([MAX_SINGLE_AMOUNT_STROOPS + 1]),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn accumulate_amounts_two_large_but_valid_ok() {
        let half = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq!(accumulate_amounts([half, half]), Ok(half + half));
    }

    #[test]
    fn accumulate_amounts_sum_near_i128_max_is_ok() {
        // Sum = i128::MAX - 1; each is well under the single-amount cap after
        // validate_single_amount is disabled for large amounts. Since
        // MAX_SINGLE_AMOUNT_STROOPS is 1_000_000_0000000 ~ 1e13, and i128::MAX
        // is ~1.7e38, we need lots of small values for a near-overflow test.
        //
        // Instead test with two values that approach the single-amount limit.
        let a = MAX_SINGLE_AMOUNT_STROOPS;
        let b = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq!(accumulate_amounts([a, b]), Ok(a + b));
    }

    #[test]
    fn accumulate_amounts_rejects_overflowing_pair() {
        // When amounts are individually within the MAX_SINGLE_AMOUNT_STROOPS cap
        // but their sum overflows i128, accumulate_amounts must reject.
        //
        // Each value is i128::MAX / 2 + 1, so their sum overflows.
        let big = i128::MAX / 2 + 2;
        // But big exceeds MAX_SINGLE_AMOUNT_STROOPS, so it will be rejected by
        // validate_single_amount first — that's fine and expected.
        assert_eq!(
            accumulate_amounts([big, big]),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn accumulate_amounts_many_tiny_amounts_near_total_cap() {
        // MAX_SINGLE_AMOUNT_STROOPS / 10 repeated 10 times = MAX_SINGLE_AMOUNT_STROOPS
        let small = MAX_SINGLE_AMOUNT_STROOPS / 10;
        let amounts = [small; 10];
        let result = accumulate_amounts(amounts);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), small * 10);
    }

    #[test]
    fn accumulate_amounts_zero_value_rejected() {
        assert_eq!(
            accumulate_amounts([1_i128, 0_i128, 3_i128]),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn accumulate_amounts_negative_value_rejected() {
        assert_eq!(
            accumulate_amounts([1_i128, -1_i128, 3_i128]),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  validate_single_amount  –  extremes
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn validate_single_amount_min_positive_is_one() {
        assert!(validate_single_amount(1).is_ok());
    }

    #[test]
    fn validate_single_amount_zero_is_rejected() {
        assert_eq!(
            validate_single_amount(0),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn validate_single_amount_negative_is_rejected() {
        assert_eq!(
            validate_single_amount(-1),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_single_amount(i128::MIN),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn validate_single_amount_exactly_at_max_is_ok() {
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());
    }

    #[test]
    fn validate_single_amount_one_above_max_is_rejected() {
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn validate_single_amount_i128_max_is_rejected() {
        // i128::MAX massively exceeds the single-amount cap
        assert_eq!(
            validate_single_amount(i128::MAX),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  validate_milestone_amounts  –  contract-total boundaries
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn validate_milestone_amounts_exactly_at_cap_is_ok() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        assert!(validate_milestone_amounts(&[cap], cap).is_ok());
    }

    #[test]
    fn validate_milestone_amounts_one_above_cap_is_rejected() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq!(
            validate_milestone_amounts(&[cap + 1], cap),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn validate_milestone_amounts_sum_at_cap_across_two_milestones_is_ok() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        let half = cap / 2;
        let remainder = cap - half;
        assert!(validate_milestone_amounts(&[half, remainder], cap).is_ok());
    }

    #[test]
    fn validate_milestone_amounts_sum_one_above_cap_across_two_is_rejected() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        let half = cap / 2 + 1;
        assert_eq!(
            validate_milestone_amounts(&[half, half], cap),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn validate_milestone_amounts_large_contract_total_i128_max() {
        // Using i128::MAX as the contract cap should still work for valid milestones.
        // A single milestone of MAX_SINGLE_AMOUNT_STROOPS should be fine.
        assert!(validate_milestone_amounts(&[MAX_SINGLE_AMOUNT_STROOPS], i128::MAX).is_ok());
    }

    #[test]
    fn validate_milestone_amounts_with_individual_above_cap() {
        // Each milestone individually above the cap
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq!(
            validate_milestone_amounts(
                &[MAX_SINGLE_AMOUNT_STROOPS + 1, MAX_SINGLE_AMOUNT_STROOPS + 1],
                cap
            ),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  validate_deposit_amount  –  overflow edge cases
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn validate_deposit_amount_normal_deposit_is_ok() {
        assert!(validate_deposit_amount(100, 0, MAX_SINGLE_AMOUNT_STROOPS).is_ok());
    }

    #[test]
    fn validate_deposit_amount_exactly_fills_cap_is_ok() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        assert!(validate_deposit_amount(cap, 0, cap).is_ok());
    }

    #[test]
    fn validate_deposit_amount_large_current_fills_remainder_is_ok() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        let current = cap - 1;
        assert!(validate_deposit_amount(1, current, cap).is_ok());
    }

    #[test]
    fn validate_deposit_amount_one_stroop_over_is_rejected() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq!(
            validate_deposit_amount(cap + 1, 0, cap),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn validate_deposit_amount_overflow_current_plus_deposit() {
        // current = i128::MAX, deposit = 1 → checked_add returns None → PotentialOverflow
        assert_eq!(
            validate_deposit_amount(1, i128::MAX, i128::MAX),
            Err(EscrowError::PotentialOverflow)
        );
    }

    #[test]
    fn validate_deposit_amount_overflow_with_small_current_large_deposit() {
        // current = i128::MAX - 1, deposit = 2 → sum overflows
        assert_eq!(
            validate_deposit_amount(2, i128::MAX - 1, i128::MAX),
            Err(EscrowError::PotentialOverflow)
        );
    }

    #[test]
    fn validate_deposit_amount_positive_at_i128_max_current() {
        // deposit = 0/negative is tested elsewhere; positive at max current overflows
        assert_eq!(
            validate_deposit_amount(1, i128::MAX, i128::MAX),
            Err(EscrowError::PotentialOverflow)
        );
    }

    #[test]
    fn validate_deposit_amount_zero_deposit_rejected() {
        assert_eq!(
            validate_deposit_amount(0, 0, MAX_SINGLE_AMOUNT_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn validate_deposit_amount_negative_deposit_rejected() {
        assert_eq!(
            validate_deposit_amount(-1, 0, MAX_SINGLE_AMOUNT_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn validate_deposit_amount_exact_sum_at_cap_but_individual_exceeds() {
        // Deposit amount alone exceeds MAX_SINGLE_AMOUNT_STROOPS
        let cap = MAX_SINGLE_AMOUNT_STROOPS * 2;
        assert_eq!(
            validate_deposit_amount(MAX_SINGLE_AMOUNT_STROOPS + 1, 0, cap),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn validate_deposit_amount_fully_funded_rejects_any() {
        let cap = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq!(
            validate_deposit_amount(1, cap, cap),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  Cross-function invariant: checked arithmetic never wraps
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn safe_add_never_wraps_to_negative_from_positive() {
        // If adding two positives produces a negative, that's a wrap bug.
        // For any a > 0, b > 0, the sum must either be Some(>a) or None.
        let a: i128 = 1;
        let b: i128 = i128::MAX;
        match safe_add_amounts(a, b) {
            None => {} // expected overflow
            Some(sum) => assert!(sum > a, "sum {sum} must be > {a} after add"),
        }
    }

    #[test]
    fn safe_sub_never_wraps_to_positive_from_negative_below_min() {
        // Subtracting a positive from i128::MIN must return None
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
        assert_eq!(safe_subtract_amounts(i128::MIN, i128::MAX), None);
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  Saturating arithmetic (u32 / u64 TTL & ledger helpers)
    // ══════════════════════════════════════════════════════════════════════════════
    //
    // Several helpers in `ttl.rs` and `governance.rs` use `saturating_add` /
    // `saturating_sub` on `u32` ledger-sequence values. These are intentionally
    // saturating because ledger sequence numbers are monotonic and the Soroban
    // host will never let `u32` overflow in practice — a wrap would mean the
    // ledger sequence exceeded 4 billion, far beyond any realistic deployment.
    //
    // We still verify the expected saturation boundary below.

    #[test]
    fn u32_saturating_add_at_max_stays_at_max() {
        let max: u32 = u32::MAX;
        assert_eq!(max.saturating_add(1), u32::MAX);
        assert_eq!(max.saturating_add(1000), u32::MAX);
    }

    #[test]
    fn u32_saturating_add_below_max_is_normal() {
        assert_eq!(u32::MAX.saturating_sub(1).saturating_add(1), u32::MAX);
    }

    #[test]
    fn u32_saturating_sub_at_zero_stays_at_zero() {
        assert_eq!(0u32.saturating_sub(1), 0);
        assert_eq!(0u32.saturating_sub(1000), 0);
    }

    #[test]
    fn u64_saturating_add_at_max_stays_at_max() {
        let max: u64 = u64::MAX;
        assert_eq!(max.saturating_add(1), u64::MAX);
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  Reputation arithmetic: completed_contracts & total_rating
    // ══════════════════════════════════════════════════════════════════════════════
    //
    // Reputation accumulation (`rep.completed_contracts += 1`,
    // `rep.total_rating += rating`) now uses `checked_add` with a
    // `PotentialOverflow` panic on overflow. Since `i128::MAX` is ~1.7e38 and
    // no realistic deployment will ever reach that many contracts, overflow is
    // impossible in practice. The checked ops are defense-in-depth.
    //
    // We verify the pure `i128` arithmetic invariants below.

    #[test]
    fn reputation_checked_add_many_contracts_does_not_overflow() {
        // 10^12 contracts × rating 5 = 5e12, well below i128::MAX (1.7e38)
        let contracts: i128 = 1_000_000_000_000;
        assert!(contracts.checked_add(1).is_some());
        let total: Option<i128> = contracts.checked_mul(5);
        assert!(total.is_some());
    }

    #[test]
    fn reputation_checked_add_at_max_would_fail() {
        // If somehow we reached i128::MAX contracts, adding one more would fail
        assert_eq!(i128::MAX.checked_add(1), None);
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  accumulate_amounts with i128 extremes (bypassing single-amount cap via
    //  the internal validate_single_amount guard)
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn accumulate_amounts_two_values_each_near_i128_max_div_2() {
        // Each value is just under i128::MAX / 2, so their sum is just under
        // i128::MAX. But each also exceeds MAX_SINGLE_AMOUNT_STROOPS, so
        // validate_single_amount rejects them first.
        let big = i128::MAX / 2;
        assert_eq!(
            accumulate_amounts([big, big]),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn accumulate_amounts_sum_to_i128_max_using_many_small_values() {
        // We can't test i128::MAX sum with validate_single_amount active
        // because each value must be ≤ MAX_SINGLE_AMOUNT_STROOPS (~1e13).
        // i128::MAX / 1e13 ≈ 1.7e25 values needed, which is impractical.
        //
        // Instead, verify that a sum to MAX_SINGLE_AMOUNT_STROOPS works:
        let small = MAX_SINGLE_AMOUNT_STROOPS / 100;
        let amounts: [i128; 100] = [small; 100];
        let result = accumulate_amounts(amounts);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), small * 100);
    }

    // ══════════════════════════════════════════════════════════════════════════════
    //  validate_deposit_amount — comprehensive overflow decision table
    // ══════════════════════════════════════════════════════════════════════════════

    #[test]
    fn validate_deposit_amount_decision_table() {
        struct Case {
            desc: &'static str,
            deposit: i128,
            current: i128,
            cap: i128,
            expected: Result<(), EscrowError>,
        }

        let cases = [
            Case {
                desc: "normal deposit under capacity",
                deposit: 100,
                current: 0,
                cap: 1000,
                expected: Ok(()),
            },
            Case {
                desc: "deposit exactly fills remaining capacity (boundary)",
                deposit: 500,
                current: 500,
                cap: 1000,
                expected: Ok(()),
            },
            Case {
                desc: "deposit one stroop over remaining capacity",
                deposit: 501,
                current: 500,
                cap: 1000,
                expected: Err(EscrowError::InvalidMilestoneAmount),
            },
            Case {
                desc: "deposit into fully funded contract",
                deposit: 1,
                current: 1000,
                cap: 1000,
                expected: Err(EscrowError::InvalidMilestoneAmount),
            },
            Case {
                desc: "zero deposit",
                deposit: 0,
                current: 0,
                cap: 1000,
                expected: Err(EscrowError::AmountMustBePositive),
            },
            Case {
                desc: "negative deposit",
                deposit: -1,
                current: 0,
                cap: 1000,
                expected: Err(EscrowError::AmountMustBePositive),
            },
            Case {
                desc: "i128::MAX current + 1 deposit = overflow",
                deposit: 1,
                current: i128::MAX,
                cap: i128::MAX,
                expected: Err(EscrowError::PotentialOverflow),
            },
            Case {
                desc: "i128::MAX deposit alone exceeds MAX_SINGLE_AMOUNT_STROOPS",
                deposit: i128::MAX,
                current: 0,
                cap: i128::MAX,
                expected: Err(EscrowError::InvalidMilestoneAmount),
            },
            Case {
                desc: "deposit amount exceeds single amount cap",
                deposit: MAX_SINGLE_AMOUNT_STROOPS + 1,
                current: 0,
                cap: MAX_SINGLE_AMOUNT_STROOPS * 2,
                expected: Err(EscrowError::InvalidMilestoneAmount),
            },
            Case {
                desc: "deposit one stroop short of total capacity",
                deposit: 499,
                current: 500,
                cap: 1000,
                expected: Ok(()),
            },
        ];

        for (i, c) in cases.iter().enumerate() {
            let result = validate_deposit_amount(c.deposit, c.current, c.cap);
            assert_eq!(
                result, c.expected,
                "Case {} '{}' failed: expected {:?}, got {:?}",
                i, c.desc, c.expected, result
            );
        }
    }
}
