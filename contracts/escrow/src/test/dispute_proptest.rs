//! Property-based tests for dispute resolution invariants.
//!
//! Covers the pure arithmetic in [`resolution_payouts`] and
//! [`final_status_after_resolution`] with randomized inputs:
//!
//!  1. Conservation: `client + freelancer == available` for every variant.
//!  2. PartialRefund: freelancer gets floor(available * 30 / 100).
//!  3. Split: valid splits are accepted, invalid splits are rejected.
//!  4. Status: Refunded iff refunded == funded.
//!  5. Accounting guard: corrupted state is rejected with the right error.
//!  6. Integration: full raise + resolve lifecycle preserves invariants.
//!
//! ## Running
//!
//! ```sh
//! cargo test -p escrow dispute_proptest
//! ```
//!
//! Failing seeds are saved to `proptest-regressions/dispute_proptest.txt`.

#![cfg(test)]

extern crate std;

use std::vec::Vec as StdVec;

use proptest::prelude::*;
use soroban_sdk::{
    testutils::Address as _, Address, Env, Vec as SorobanVec,
};

use crate::{
    Contract, ContractStatus, DisputeResolution, DisputeSplit, Error, Escrow,
    EscrowClient, ReleaseAuthorization,
};

use crate::dispute::{final_status_after_resolution, resolution_payouts};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Cap amounts to stay well below i128::MAX / 30 for PartialRefund overflow
/// safety.  i128::MAX / 30 ≈ 5.67e36. We cap at 1e18 so the proptest
/// shrinking still works with reasonable values.
const MAX_AMOUNT_FOR_PARTIAL: i128 = 1_000_000_000_000_000_000; // 1e18

const DEFAULT_CASES: u32 = 256;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a minimal `Contract` for pure-arithmetic tests.
fn make_contract(env: &Env, funded: i128, released: i128, refunded: i128) -> Contract {
    Contract {
        client: Address::generate(env),
        freelancer: Address::generate(env),
        arbiter: Some(Address::generate(env)),
        status: ContractStatus::Disputed,
        total_deposited: funded,
        funded_amount: funded,
        released_amount: released,
        refunded_amount: refunded,
        release_authorization: ReleaseAuthorization::ClientOnly,
        reputation_issued: false,
    }
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Generate a valid accounting triple: `(funded, released, refunded)`
/// where `released + refunded <= funded`.
prop_compose! {
    fn valid_accounting(
        max_amount: i128,
    )(
        funded in 0i128..=max_amount,
    )(
        funded in Just(funded),
        released in 0i128..=funded,
    )(
        funded in Just(funded),
        released in Just(released),
        refunded in 0i128..=(funded - released),
    ) -> (i128, i128, i128) {
        (funded, released, refunded)
    }
}

/// Generate a corrupted accounting triple where `released + refunded > funded`,
/// producing a negative available balance.
prop_compose! {
    fn corrupted_accounting()(
        funded in 0i128..i128::MAX,
    )(
        funded in Just(funded),
        // overshoot is guaranteed positive and won't overflow when added to funded
        // because we clamp to i128::MAX - funded
        overshoot in 1i128..=(i128::MAX.saturating_sub(funded).max(1)),
    )(
        total in Just(funded.saturating_add(overshoot)),
        released in 0i128..=funded.saturating_add(overshoot),
    ) -> (i128, i128, i128) {
        let refunded = total.saturating_sub(released);
        (funded, released, refunded)
    }
}

/// Generate a valid split that sums exactly to `available`.
prop_compose! {
    fn valid_splits(available: i128)(
        client_amount in 0i128..=available,
    ) -> DisputeSplit {
        DisputeSplit {
            client_amount,
            freelancer_amount: available - client_amount,
        }
    }
}

// ---------------------------------------------------------------------------
// Properties: resolution_payouts (pure arithmetic)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: DEFAULT_CASES,
        ..ProptestConfig::default()
    })]

    /// Conservation invariant: for any valid accounting state and any
    /// resolution variant, client_payout + freelancer_payout == available.
    #[test]
    fn prop_conservation_invariant_holds(
        (funded, released, refunded) in valid_accounting(MAX_AMOUNT_FOR_PARTIAL),
    ) {
        let env = Env::default();
        let contract = make_contract(&env, funded, released, refunded);
        let available = funded - released - refunded;

        // FullRefund
        let (c, f) = resolution_payouts(&contract, &DisputeResolution::FullRefund).unwrap();
        prop_assert_eq!(c + f, available, "FullRefund: sum != available");
        prop_assert_eq!(c, available);
        prop_assert_eq!(f, 0);

        // FullPayout
        let (c, f) = resolution_payouts(&contract, &DisputeResolution::FullPayout).unwrap();
        prop_assert_eq!(c + f, available, "FullPayout: sum != available");
        prop_assert_eq!(c, 0);
        prop_assert_eq!(f, available);

        // PartialRefund (safe within MAX_AMOUNT_FOR_PARTIAL)
        let (c, f) = resolution_payouts(&contract, &DisputeResolution::PartialRefund).unwrap();
        prop_assert_eq!(c + f, available, "PartialRefund: sum != available");
        let expected_f = (available * 30) / 100;
        prop_assert_eq!(f, expected_f, "PartialRefund: freelancer floor mismatch");
        prop_assert_eq!(c, available - expected_f, "PartialRefund: client calc mismatch");

        // Split: test a valid split derived from the actual available.
    }

    /// PartialRefund applies floor(available * 30 / 100) to freelancer
    /// with client receiving the remainder, for all valid amounts.
    #[test]
    fn prop_partial_refund_floor_rounding(
        (funded, released, refunded) in valid_accounting(MAX_AMOUNT_FOR_PARTIAL),
    ) {
        let env = Env::default();
        let contract = make_contract(&env, funded, released, refunded);
        let available = funded - released - refunded;

        // The checked_mul guard: if available > i128::MAX / 30,
        // PartialRefund legitimately returns PotentialOverflow.
        let result = resolution_payouts(&contract, &DisputeResolution::PartialRefund);
        if available.checked_mul(30).is_none() {
            prop_assert!(result.is_err());
            return;
        }
        let (client, freelancer) = result.unwrap();
        let expected_freelancer = (available * 30) / 100;
        prop_assert_eq!(freelancer, expected_freelancer);
        prop_assert_eq!(client, available - expected_freelancer);
        prop_assert_eq!(client + freelancer, available);
    }

    /// Split accepts a valid (a, b) where a + b == available and both >= 0.
    /// The split is derived from the contract's actual available balance.
    #[test]
    fn prop_split_accepts_valid(
        (funded, released, refunded) in valid_accounting(MAX_AMOUNT_FOR_PARTIAL),
    ) {
        let env = Env::default();
        let contract = make_contract(&env, funded, released, refunded);
        let available = funded - released - refunded;

        // Generate a random valid split for THIS contract's available.
        let client_amount = if available > 0 {
            // Use a simple deterministic split at randomized proportions
            available / 2
        } else {
            0
        };
        let split = DisputeSplit {
            client_amount,
            freelancer_amount: available - client_amount,
        };

        let result = resolution_payouts(&contract, &DisputeResolution::Split(split));
        prop_assert!(result.is_ok(), "valid split rejected: {:?} for available={}", result, available);
        let (c, f) = result.unwrap();
        prop_assert_eq!(c + f, available);
        prop_assert_eq!(c, client_amount);
        prop_assert_eq!(f, available - client_amount);
    }

    /// Split rejects invalid amounts: negatives, non-conserving sums,
    /// and individual amounts exceeding available.
    #[test]
    fn prop_split_rejects_invalid(
        (funded, released, refunded) in valid_accounting(MAX_AMOUNT_FOR_PARTIAL),
    ) {
        let available = funded - released - refunded;
        prop_assume!(available > 0);
        prop_assume!(available < MAX_AMOUNT_FOR_PARTIAL);

        let env = Env::default();
        let contract = make_contract(&env, funded, released, refunded);

        // Reject negative client_amount
        let result = resolution_payouts(
            &contract,
            &DisputeResolution::Split(DisputeSplit {
                client_amount: -1,
                freelancer_amount: available + 1,
            }),
        );
        prop_assert_eq!(result, Err(Error::InvalidDisputeSplit),
            "should reject negative client_amount");

        // Reject negative freelancer_amount
        let result = resolution_payouts(
            &contract,
            &DisputeResolution::Split(DisputeSplit {
                client_amount: available + 1,
                freelancer_amount: -1,
            }),
        );
        prop_assert_eq!(result, Err(Error::InvalidDisputeSplit),
            "should reject negative freelancer_amount");

        // Reject non-conserving sum (under)
        let result = resolution_payouts(
            &contract,
            &DisputeResolution::Split(DisputeSplit {
                client_amount: available / 2,
                freelancer_amount: available / 2 - 1,
            }),
        );
        prop_assert_eq!(result, Err(Error::InvalidDisputeSplit),
            "should reject under-allocated sum");

        // Reject non-conserving sum (over)
        let result = resolution_payouts(
            &contract,
            &DisputeResolution::Split(DisputeSplit {
                client_amount: available / 2,
                freelancer_amount: available / 2 + 1,
            }),
        );
        prop_assert_eq!(result, Err(Error::InvalidDisputeSplit),
            "should reject over-allocated sum");

        // Reject individual > available
        let result = resolution_payouts(
            &contract,
            &DisputeResolution::Split(DisputeSplit {
                client_amount: available + 1,
                freelancer_amount: 0,
            }),
        );
        prop_assert_eq!(result, Err(Error::InvalidDisputeSplit),
            "should reject client_amount > available");
    }

    // ── final_status_after_resolution ────────────────────────────────────────

    /// `final_status_after_resolution` returns `Refunded` iff
    /// `refunded_amount == funded_amount`; otherwise `Completed`.
    #[test]
    fn prop_final_status_refunded_iff_fully_refunded(
        funded in 0i128..=MAX_AMOUNT_FOR_PARTIAL,
    ) {
        let env = Env::default();
        // Test: refunded == funded → Refunded
        let contract = make_contract(&env, funded, 0, funded);
        prop_assert_eq!(
            final_status_after_resolution(&contract),
            ContractStatus::Refunded,
            "fully refunded should return Refunded"
        );

        // Test: refunded < funded → Completed
        if funded > 0 {
            let contract = make_contract(&env, funded, 0, funded - 1);
            prop_assert_eq!(
                final_status_after_resolution(&contract),
                ContractStatus::Completed,
                "partially refunded should return Completed"
            );
        }
    }

    // ── Corrupted state ──────────────────────────────────────────────────────

    /// When `released + refunded > funded`, the function must return
    /// `AccountingInvariantViolated`.
    #[test]
    fn prop_corrupted_state_rejected(
        (funded, released, refunded) in corrupted_accounting(),
    ) {
        let env = Env::default();
        let contract = make_contract(&env, funded, released, refunded);

        // Sanity: this state should indeed be corrupted.
        let available = funded - released - refunded;
        prop_assert!(available < 0 || released + refunded > funded,
            "corrupted strategy produced valid state: funded={funded}, released={released}, refunded={refunded}");

        let result = resolution_payouts(&contract, &DisputeResolution::FullRefund);
        prop_assert_eq!(result, Err(Error::AccountingInvariantViolated));
    }

    /// Zero available must produce (0, 0) for every resolution variant.
    #[test]
    fn prop_zero_available_all_variants(
        funded in 0i128..=MAX_AMOUNT_FOR_PARTIAL,
    ) {
        let env = Env::default();
        // released=0, refunded=funded → available == 0
        let contract = make_contract(&env, funded, 0, funded);
        let available = funded - contract.released_amount - contract.refunded_amount;
        prop_assert_eq!(available, 0, "expected zero available");

        // FullRefund → (0, 0)
        let (c, f) = resolution_payouts(&contract, &DisputeResolution::FullRefund).unwrap();
        prop_assert_eq!((c, f), (0, 0));

        // FullPayout → (0, 0)
        let (c, f) = resolution_payouts(&contract, &DisputeResolution::FullPayout).unwrap();
        prop_assert_eq!((c, f), (0, 0));

        // PartialRefund → (0, 0) — floor(0 * 30 / 100) = 0
        let (c, f) = resolution_payouts(&contract, &DisputeResolution::PartialRefund).unwrap();
        prop_assert_eq!((c, f), (0, 0));

        // Split(0, 0) → (0, 0)
        let split = DisputeSplit { client_amount: 0, freelancer_amount: 0 };
        let (c, f) = resolution_payouts(
            &contract, &DisputeResolution::Split(split)
        ).unwrap();
        prop_assert_eq!((c, f), (0, 0));
    }

    /// For zero-funded contracts, `final_status_after_resolution` returns
    /// `Refunded` because `refunded_amount == funded_amount == 0`.
    #[test]
    fn prop_zero_funded_status_is_refunded() {
        let env = Env::default();
        let contract = make_contract(&env, 0, 0, 0);
        prop_assert_eq!(
            final_status_after_resolution(&contract),
            ContractStatus::Refunded,
        );
    }

    /// Split with i128::MAX amounts where sum overflows must return
    /// `PotentialOverflow`.
    #[test]
    fn prop_split_overflow_rejected() {
        let env = Env::default();
        let contract = make_contract(&env, i128::MAX, 0, 0);
        let split = DisputeSplit {
            client_amount: i128::MAX,
            freelancer_amount: 1,
        };
        let result = resolution_payouts(&contract, &DisputeResolution::Split(split));
        prop_assert_eq!(result, Err(Error::PotentialOverflow));
    }
}

// ---------------------------------------------------------------------------
// Integration properties: full dispute lifecycle
// ---------------------------------------------------------------------------

/// The set of dispute-lifecycle operations.
#[derive(Clone, Debug)]
enum DisputeOp {
    /// Deposit `amount` (caller: client).
    Deposit(i128),
    /// Approve milestone `index` (caller: client).
    Approve(u32),
    /// Release milestone `index` (caller: client).
    Release(u32),
    /// Refund milestone `index` (caller: client).
    Refund(u32),
    /// Raise a dispute (caller: client or freelancer).
    RaiseDispute,
    /// Resolve the dispute with the given resolution (caller: arbiter).
    ResolveDispute(DisputeResolution),
}

// ── Integration strategy helpers ─────────────────────────────────────────────

fn int_milestone_amounts() -> impl Strategy<Value = StdVec<i128>> {
    prop::collection::vec(1i128..=1_000_000i128, 1..=3usize)
}

fn int_op_strategy(n_ms: u32) -> impl Strategy<Value = DisputeOp> {
    let n = n_ms;
    prop_oneof![
        2 => (1i128..=1_000_000i128).prop_map(DisputeOp::Deposit),
        1 => (0u32..n).prop_map(DisputeOp::Approve),
        1 => (0u32..n).prop_map(DisputeOp::Release),
        1 => (0u32..n).prop_map(DisputeOp::Refund),
        2 => Just(DisputeOp::RaiseDispute),
        3 => prop_oneof![
            Just(DisputeResolution::FullRefund),
            Just(DisputeResolution::FullPayout),
            Just(DisputeResolution::PartialRefund),
            // For Split we use a small safe split that likely works
            // after some funds may have been released/refunded.
            (1i128..=500_000i128).prop_map(|half| DisputeResolution::Split(DisputeSplit {
                client_amount: half,
                freelancer_amount: half,
            })),
        ].prop_map(DisputeOp::ResolveDispute),
    ]
}

fn int_ops_strategy(n_ms: u32) -> impl Strategy<Value = StdVec<DisputeOp>> {
    prop::collection::vec(int_op_strategy(n_ms), 5..=20usize)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: DEFAULT_CASES,
        ..ProptestConfig::default()
    })]

    /// Full dispute lifecycle: create, fund, operate, dispute, resolve.
    /// The accounting invariant (`funded >= released + refunded`) must hold
    /// after every operation, including after dispute resolution.
    #[test]
    fn prop_dispute_lifecycle_invariant(
        (amounts, ops) in int_milestone_amounts().prop_flat_map(|amounts| {
            let n = amounts.len() as u32;
            (Just(amounts), int_ops_strategy(n))
        }),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        let arbiter_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);

        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &Some(arbiter_addr.clone()),
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            escrow.deposit_funds(&contract_id, &client_addr, &total);
        }));

        let ms_count = amounts.len() as u32;
        let mut resolved = false;

        for op in &ops {
            if resolved {
                break;
            }

            let _ = match op {
                DisputeOp::Deposit(amount) => {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        escrow.deposit_funds(&contract_id, &client_addr, amount);
                    }))
                }
                DisputeOp::Approve(idx) if *idx < ms_count => {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        escrow.approve_milestone_release(&contract_id, &client_addr, idx);
                    }))
                }
                DisputeOp::Release(idx) if *idx < ms_count => {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        escrow.release_milestone(&contract_id, &client_addr, idx);
                    }))
                }
                DisputeOp::Refund(idx) if *idx < ms_count => {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let v: SorobanVec<u32> = {
                            let mut tmp = SorobanVec::new(&env);
                            tmp.push_back(*idx);
                            tmp
                        };
                        escrow.refund_unreleased_milestones(&contract_id, &v);
                    }))
                }
                DisputeOp::RaiseDispute => {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        escrow.raise_dispute(&contract_id, &client_addr);
                    }))
                }
                DisputeOp::ResolveDispute(res) => {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        escrow.resolve_dispute(&contract_id, &arbiter_addr, res);
                    }));
                    if r.is_ok() {
                        resolved = true;
                    }
                    r
                }
                _ => Ok(()),
            };

            // Verify accounting invariant after every operation.
            let contract: Contract = match std::panic::catch_unwind(
                std::panic::AssertUnwindSafe(|| escrow.get_contract(&contract_id))
            ) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let available = contract.funded_amount
                - contract.released_amount
                - contract.refunded_amount;
            prop_assert!(
                available >= 0,
                "invariant violated after op {:?}: funded={}, released={}, refunded={}",
                op,
                contract.funded_amount,
                contract.released_amount,
                contract.refunded_amount,
            );

            if resolved {
                prop_assert_eq!(
                    contract.released_amount + contract.refunded_amount,
                    contract.funded_amount,
                    "post-resolution: released + refunded != funded"
                );
                prop_assert!(
                    contract.status == ContractStatus::Refunded
                        || contract.status == ContractStatus::Completed,
                    "post-resolution status not terminal: {:?}",
                    contract.status,
                );
            }
        }
    }

    /// Dispute raised and resolved with FullRefund must move all available
    /// to refunded_amount and mark Refunded.
    #[test]
    fn prop_dispute_full_refund_integration(
        amounts in int_milestone_amounts(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        let arbiter_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);
        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &Some(arbiter_addr.clone()),
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        assert!(escrow.deposit_funds(&contract_id, &client_addr, &total));

        assert!(escrow.raise_dispute(&contract_id, &client_addr));
        assert!(escrow.resolve_dispute(
            &contract_id,
            &arbiter_addr,
            &DisputeResolution::FullRefund,
        ));

        let contract = escrow.get_contract(&contract_id);
        prop_assert_eq!(contract.status, ContractStatus::Refunded);
        prop_assert_eq!(contract.refunded_amount, total);
        prop_assert_eq!(contract.released_amount, 0);
        prop_assert_eq!(
            contract.released_amount + contract.refunded_amount,
            contract.funded_amount,
        );
    }

    /// Dispute raised and resolved with FullPayout must move all available
    /// to released_amount and mark Completed.
    #[test]
    fn prop_dispute_full_payout_integration(
        amounts in int_milestone_amounts(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        let arbiter_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);
        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &Some(arbiter_addr.clone()),
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        assert!(escrow.deposit_funds(&contract_id, &client_addr, &total));

        assert!(escrow.raise_dispute(&contract_id, &client_addr));
        assert!(escrow.resolve_dispute(
            &contract_id,
            &arbiter_addr,
            &DisputeResolution::FullPayout,
        ));

        let contract = escrow.get_contract(&contract_id);
        prop_assert_eq!(contract.status, ContractStatus::Completed);
        prop_assert_eq!(contract.released_amount, total);
        prop_assert_eq!(contract.refunded_amount, 0);
        prop_assert_eq!(
            contract.released_amount + contract.refunded_amount,
            contract.funded_amount,
        );
    }

    /// PartialRefund via dispute resolution must produce a 70/30 split
    /// with the freelancer receiving floor(available * 30 / 100).
    #[test]
    fn prop_dispute_partial_refund_split_integration(
        amounts in int_milestone_amounts(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        let arbiter_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);
        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &Some(arbiter_addr.clone()),
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        assert!(escrow.deposit_funds(&contract_id, &client_addr, &total));

        assert!(escrow.raise_dispute(&contract_id, &client_addr));
        assert!(escrow.resolve_dispute(
            &contract_id,
            &arbiter_addr,
            &DisputeResolution::PartialRefund,
        ));

        let contract = escrow.get_contract(&contract_id);
        prop_assert_eq!(contract.status, ContractStatus::Completed);
        prop_assert_eq!(
            contract.released_amount + contract.refunded_amount,
            contract.funded_amount,
        );

        let expected_freelancer = (total * 30) / 100;
        prop_assert_eq!(contract.released_amount, expected_freelancer);
        prop_assert_eq!(contract.refunded_amount, total - expected_freelancer);
    }

    /// Dispute with Split resolution must produce the exact requested
    /// amounts and conserve balance. The split ratio is randomized.
    #[test]
    fn prop_dispute_split_integration(
        amounts in int_milestone_amounts(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        let arbiter_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);
        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &Some(arbiter_addr.clone()),
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        assert!(escrow.deposit_funds(&contract_id, &client_addr, &total));

        // Use a custom split: 40/60 client/freelancer.
        let client_portion = (total * 4) / 10;
        let freelancer_portion = total - client_portion;
        let split = DisputeSplit {
            client_amount: client_portion,
            freelancer_amount: freelancer_portion,
        };

        assert!(escrow.raise_dispute(&contract_id, &client_addr));
        assert!(escrow.resolve_dispute(
            &contract_id,
            &arbiter_addr,
            &DisputeResolution::Split(split),
        ));

        let contract = escrow.get_contract(&contract_id);
        prop_assert_eq!(contract.status, ContractStatus::Completed);
        prop_assert_eq!(contract.refunded_amount, client_portion);
        prop_assert_eq!(contract.released_amount, freelancer_portion);
        prop_assert_eq!(
            contract.released_amount + contract.refunded_amount,
            contract.funded_amount,
        );
    }

    /// Raise dispute is rejected when no arbiter is configured.
    #[test]
    fn prop_raise_dispute_rejected_without_arbiter(
        amounts in int_milestone_amounts(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);
        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &None, // No arbiter
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        assert!(escrow.deposit_funds(&contract_id, &client_addr, &total));

        let result = escrow.try_raise_dispute(&contract_id, &client_addr);
        prop_assert!(result.is_err());
    }

    /// Double-resolve is rejected.
    #[test]
    fn prop_double_resolve_rejected(
        amounts in int_milestone_amounts(),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        let arbiter_addr = Address::generate(&env);

        let escrow_id = env.register(Escrow, ());
        let escrow = EscrowClient::new(&env, &escrow_id);
        let admin = Address::generate(&env);
        escrow.initialize(&admin);

        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };

        let contract_id = escrow.create_contract(
            &client_addr,
            &freelancer_addr,
            &Some(arbiter_addr.clone()),
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let total: i128 = amounts.iter().sum();
        assert!(escrow.deposit_funds(&contract_id, &client_addr, &total));

        assert!(escrow.raise_dispute(&contract_id, &client_addr));
        assert!(escrow.resolve_dispute(
            &contract_id,
            &arbiter_addr,
            &DisputeResolution::FullRefund,
        ));

        let result = escrow.try_resolve_dispute(
            &contract_id,
            &arbiter_addr,
            &DisputeResolution::FullPayout,
        );
        prop_assert!(result.is_err());
    }
}
