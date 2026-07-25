//! Dispute resolution payout arithmetic tests.
//!
//! These tests verify the pure money-splitting logic in `resolution_payouts`:
//!
//!   - FullRefund: all available → client (available, 0)
//!   - FullPayout: all available → freelancer (0, available)
//!   - PartialRefund: 70/30 split with floor rounding on freelancer leg
//!   - Split: custom split requiring sum == available, no negative amounts
//!
//! Conservation invariant: client_payout + freelancer_payout == available.
//!
//! ## Lifecycle & auth tests
//!
//! The second section covers raise/resolve lifecycle and auth boundaries:
//!
//!   - Only a party to the contract may raise a dispute
//!   - Only the designated arbiter may resolve it
//!   - Resolving moves funds/state correctly
//!   - Raising in a terminal state is rejected with typed error
//!   - Non-party raise is rejected
//!   - Non-arbiter resolve is rejected
//!   - Double-resolve is rejected
//!   - Resolve-after-settle is rejected

#![cfg(test)]

use crate::{
    ContractStatus, DisputeResolution, Escrow, EscrowClient, EscrowError, ReleaseAuthorization,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

use crate::dispute::{final_status_after_resolution, resolution_payouts};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

fn make_client(env: &Env) -> EscrowClient<'_> {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    client
}

/// Build a bare `Contract` value with controlled accounting fields for unit tests
/// that call `resolution_payouts` / `final_status_after_resolution` directly.
///
/// `funded` is stored in both `total_deposited` and `funded_amount` so the
/// helper reflects a freshly-funded contract with no prior releases.
fn payout_contract(env: &Env, funded: i128, released: i128, refunded: i128) -> Contract {
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

/// Helper: create a funded contract with an arbiter, ready for dispute.
/// Returns (client_addr, freelancer_addr, arbiter_addr, contract_id).
fn funded_contract_with_arbiter(
    env: &Env,
    client: &EscrowClient<'_>,
) -> (Address, Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let arbiter_addr = Address::generate(env);

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert!(client.deposit_funds(&contract_id, &client_addr, &deposit_amount));

    (client_addr, freelancer_addr, arbiter_addr, contract_id)
}

/// Helper: create a funded contract without an arbiter.
/// Returns (client_addr, freelancer_addr, contract_id).
fn funded_contract_no_arbiter(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = vec![env, 100_i128];
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
    (client_addr, freelancer_addr, contract_id)
}

/// Helper: create a funded contract with arbiter and raise a dispute.
/// Returns (client_addr, freelancer_addr, arbiter_addr, contract_id).
fn disputed_contract(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, Address, u32) {
    let (client_addr, freelancer_addr, arbiter_addr, contract_id) =
        funded_contract_with_arbiter(env, client);
    assert!(client.raise_dispute(&contract_id, &client_addr));
    (client_addr, freelancer_addr, arbiter_addr, contract_id)
}

// ---------------------------------------------------------------------------
// Unit tests: resolution_payouts (pure arithmetic)
// ---------------------------------------------------------------------------

#[test]
fn resolution_payouts_full_refund_routes_all_to_client() {
    let env = make_env();
    // available = 100 - 20 - 10 = 70
    let contract = payout_contract(&env, 100, 20, 10);
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::FullRefund),
        Ok((70, 0))
    );
}

#[test]
fn resolution_payouts_full_payout_routes_all_to_freelancer() {
    let env = make_env();
    let contract = payout_contract(&env, 100, 20, 10);
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::FullPayout),
        Ok((0, 70))
    );
}

/// PartialRefund applies the documented 70/30 split with floor rounding.
/// freelancer gets floor(available * 30 / 100), client gets remainder.
#[test]
fn resolution_payouts_partial_refund_applies_floor_rounded_30_pct_to_freelancer() {
    let env = make_env();
    // 101 available: freelancer = floor(101 * 30 / 100) = 30; client = 71
    let contract = payout_contract(&env, 101, 0, 0);
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::PartialRefund),
        Ok((71, 30))
    );
}

#[test]
fn resolution_payouts_split_accepts_exact_conserving_amounts() {
    let env = make_env();
    // Zero available → (0, 0)
    assert_eq!(
        resolution_payouts(
            &payout_contract(&env, 100, 0, 0),
            &DisputeResolution::Split(DisputeSplit {
                client_amount: 40,
                freelancer_amount: 60,
            })
        ),
        Ok((0, 0))
    );
    // One stroop → floor(1 * 30 / 100) = 0, client gets 1
    assert_eq!(
        resolution_payouts(
            &payout_contract(&env, 1, 0, 0),
            &DisputeResolution::PartialRefund
        ),
        Ok((1, 0))
    );
}

/// Table-driven test covering PartialRefund rounding at odd amounts.
/// Verifies that floor truncation never creates value (sum == available).
#[test]
fn resolution_payouts_partial_refund_odd_amount_rounding() {
    let env = make_env();
    // (available, expected_client, expected_freelancer)
    let cases: &[(i128, i128, i128)] = &[
        (7, 7, 0),
        (10, 7, 3),
        (99, 69, 30),
        (100, 70, 30),
        (101, 71, 30),
        (102, 71, 31),
        (103, 72, 31),
    ];
    for (available, expected_client, expected_freelancer) in cases {
        let contract = payout_contract(&env, *available, 0, 0);
        let (client, freelancer) = resolution_payouts(&contract, &DisputeResolution::PartialRefund)
            .expect("PartialRefund should not error");
        assert_eq!(
            client + freelancer,
            *available,
            "sum must equal available for amount {}",
            available
        );
        assert_eq!(client, *expected_client);
        assert_eq!(freelancer, *expected_freelancer);
    }
}

/// Split rejects negative amounts.
#[test]
fn resolution_payouts_split_rejects_negative_legs() {
    let env = make_env();
    let contract = payout_contract(&env, 100, 0, 0);
    let split = DisputeSplit {
        client_amount: -1,
        freelancer_amount: 101,
    };
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::Split(split)),
        Err(Error::InvalidDisputeSplit)
    );
    let split = DisputeSplit {
        client_amount: 101,
        freelancer_amount: -1,
    };
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::Split(split)),
        Err(Error::InvalidDisputeSplit)
    );
}

#[test]
fn resolution_payouts_split_rejects_non_conserving_sum() {
    let env = make_env();
    let contract = payout_contract(&env, 100, 0, 0);
    // 40 + 59 = 99 ≠ 100
    let split = DisputeSplit {
        client_amount: 40,
        freelancer_amount: 59,
    };
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::Split(split)),
        Err(Error::InvalidDisputeSplit)
    );
    // 40 + 61 = 101 ≠ 100
    let split = DisputeSplit {
        client_amount: 40,
        freelancer_amount: 61,
    };
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::Split(split)),
        Err(Error::InvalidDisputeSplit)
    );
}

/// Split accepts any (a, b) where a + b == available and both are non-negative.
#[test]
fn resolution_payouts_split_accepts_exact_splits() {
    let env = make_env();
    let split = DisputeSplit {
        client_amount: 40,
        freelancer_amount: 60,
    };
    assert_eq!(
        resolution_payouts(
            &payout_contract(&env, 100, 0, 0),
            &DisputeResolution::Split(split)
        ),
        Ok((40, 60))
    );
    let split = DisputeSplit {
        client_amount: 0,
        freelancer_amount: 0,
    };
    assert_eq!(
        resolution_payouts(
            &payout_contract(&env, 0, 0, 0),
            &DisputeResolution::Split(split)
        ),
        Ok((0, 0))
    );
}

/// Split uses checked addition and rejects overflow before the sum check.
#[test]
fn resolution_payouts_split_rejects_overflowing_sum() {
    let env = make_env();
    let contract = payout_contract(&env, i128::MAX, 0, 0);
    let split = DisputeSplit {
        client_amount: i128::MAX,
        freelancer_amount: 1,
    };
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::Split(split)),
        Err(Error::PotentialOverflow)
    );
}

/// Payout math fails closed when released + refunded already exceed funded_amount.
#[test]
fn resolution_payouts_rejects_corrupted_accounting_state() {
    let env = make_env();
    // released(70) + refunded(31) = 101 > funded(100) → available < 0
    let contract = payout_contract(&env, 100, 70, 31);
    assert_eq!(
        resolution_payouts(&contract, &DisputeResolution::FullRefund),
        Err(Error::AccountingInvariantViolated)
    );
}

/// Table-driven test verifying conservation invariant across all resolution variants.
#[test]
fn resolution_payouts_conserves_available_balance() {
    let env = make_env();
    let balances = &[0, 1, 2, 5, 10, 33, 99, 100, 101, 1000, 12345, 1000000];

    for &available in balances {
        let c = payout_contract(&env, available, 0, 0);

        // FullRefund
        let (client, freelancer) = resolution_payouts(&c, &DisputeResolution::FullRefund).unwrap();
        assert_eq!(client + freelancer, available);
        assert_eq!(client, available);
        assert_eq!(freelancer, 0);

        // FullPayout
        let (client, freelancer) = resolution_payouts(&c, &DisputeResolution::FullPayout).unwrap();
        assert_eq!(client + freelancer, available);
        assert_eq!(client, 0);
        assert_eq!(freelancer, available);

        // PartialRefund
        let (client, freelancer) =
            resolution_payouts(&c, &DisputeResolution::PartialRefund).unwrap();
        assert_eq!(client + freelancer, available);
        let expected_freelancer = (available * 30) / 100;
        assert_eq!(freelancer, expected_freelancer);
        assert_eq!(client, available - expected_freelancer);

        // Split (exact)
        let split_client = available / 2;
        let split_freelancer = available - split_client;
        let split = DisputeSplit {
            client_amount: split_client,
            freelancer_amount: split_freelancer,
        };
        let (client, freelancer) =
            resolution_payouts(&c, &DisputeResolution::Split(split)).unwrap();
        assert_eq!(client + freelancer, available);
        assert_eq!(client, split_client);
        assert_eq!(freelancer, split_freelancer);
    }
}

/// final_status returns Refunded only when the full deposit has been refunded.
#[test]
fn final_status_after_resolution_returns_refunded_only_when_fully_refunded() {
    let env = make_env();
    assert_eq!(
        final_status_after_resolution(&payout_contract(&env, 100, 0, 100)),
        ContractStatus::Refunded
    );
    assert_eq!(
        final_status_after_resolution(&payout_contract(&env, 100, 30, 70)),
        ContractStatus::Completed
    );
}

#[test]
fn client_can_raise_dispute_on_funded_contract() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, _, escrow_id) = create_funded_contract_with_arbiter(
        &env,
        &client,
        vec![&env, 100_i128, 200_i128],
        300_i128,
    );

    assert!(client.raise_dispute(&escrow_id, &client_addr));

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Disputed);
}

#[test]
fn freelancer_can_raise_dispute_on_funded_contract() {
    let (env, _contract_id, client) = setup_initialized();
    let (_, freelancer_addr, _, escrow_id) = create_funded_contract_with_arbiter(
        &env,
        &client,
        vec![&env, 100_i128, 200_i128],
        300_i128,
    );

    assert!(client.raise_dispute(&escrow_id, &freelancer_addr));

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Disputed);
}

/// Integration: FullRefund on a funded contract conserves balance and marks Refunded.
#[test]
fn raise_dispute_requires_contract_party() {
    let (env, _contract_id, client) = setup_initialized();
    let (_, _, _, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    let outsider = Address::generate(&env);
    super::assert_contract_error(
        client.try_raise_dispute(&escrow_id, &outsider),
        EscrowError::UnauthorizedRole,
    );
}

#[test]
fn raise_dispute_requires_assigned_arbiter() {
    let (env, _contract_id, client) = setup_initialized();
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);

    // Create contract WITHOUT arbiter
    let escrow_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert!(client.deposit_funds(&escrow_id, &client_addr, &100_i128));

    super::assert_contract_error(
        client.try_raise_dispute(&escrow_id, &client_addr),
        EscrowError::ArbiterRequired,
    );
}

#[test]
fn raise_dispute_rejects_completed_contract() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, _, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    // Release milestone and complete
    assert!(client.approve_milestone_release(&escrow_id, &client_addr, &0));
    assert!(client.release_milestone(&escrow_id, &client_addr, &0));

    super::assert_contract_error(
        client.try_raise_dispute(&escrow_id, &client_addr),
        EscrowError::InvalidState,
    );
}

#[test]
fn resolve_full_refund_marks_refunded_and_closes_accounting() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 125_i128, 75_i128], 200_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullRefund));

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Refunded);
    assert_eq!(contract.released_amount, 0);
    assert_eq!(contract.refunded_amount, 200);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

/// Integration: FullPayout on a funded contract conserves balance and marks Completed.
#[test]
fn resolve_full_payout_marks_completed_and_closes_accounting() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 150_i128], 150_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullPayout));

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(contract.released_amount, 150);
    assert_eq!(contract.refunded_amount, 0);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

/// Integration: PartialRefund applies 70/30 split and conserves balance.
#[test]
fn resolve_partial_refund_applies_70_30_split() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::PartialRefund));

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

/// Integration: Split accepts valid custom amounts and conserves balance.
#[test]
fn resolve_partial_refund_applies_to_remaining_balance() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) = create_funded_contract_with_arbiter(
        &env,
        &client,
        vec![&env, 101_i128, 100_i128],
        201_i128,
    );

    // Release first milestone
    assert!(client.approve_milestone_release(&escrow_id, &client_addr, &0));
    assert!(client.release_milestone(&escrow_id, &client_addr, &0));

    assert!(client.raise_dispute(&escrow_id, &client_addr));
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::PartialRefund));

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(contract.refunded_amount, 35);
    assert_eq!(contract.released_amount, 65);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

// ---------------------------------------------------------------------------
// Lifecycle & auth tests: raise/resolve boundaries
// ---------------------------------------------------------------------------

/// Client can raise a dispute on a funded contract with an arbiter.
#[test]
fn resolve_split_accepts_custom_amounts_that_match_available_balance() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 40_i128, 60_i128], 100_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::Split(35, 65)));

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );
}

/// Freelancer can also raise a dispute on a funded contract with an arbiter.
#[test]
fn resolve_split_rejects_invalid_totals() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));

    // Split doesn't match available balance
    super::assert_contract_error(
        client.try_resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::Split(30, 50)),
        EscrowError::InvalidDisputeSplit,
    );
}

/// A non-party (random address) cannot raise a dispute.
#[test]
fn resolve_split_rejects_negative_amounts() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));

    super::assert_contract_error(
        client.try_resolve_dispute(
            &escrow_id,
            &arbiter_addr,
            &DisputeResolution::Split(-10, 110),
        ),
        EscrowError::InvalidDisputeSplit,
    );
}

/// Raising a dispute on a contract without an arbiter is rejected.
#[test]
fn resolve_dispute_requires_assigned_arbiter() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, _, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));

    super::assert_contract_error(
        client.try_raise_dispute(&contract_id, &client_addr),
        Error::ArbiterRequired,
    );
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Funded
    );
}

/// Raising a dispute on a contract in the Completed (terminal) state is rejected
/// with a typed error.
#[test]
fn resolve_dispute_rejects_non_disputed_contract() {
    let (env, _contract_id, client) = setup_initialized();
    let (_, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    super::assert_contract_error(
        client.try_raise_dispute(&contract_id, &client_addr),
        Error::InvalidState,
    );
}

/// The designated arbiter can resolve a dispute.
#[test]
fn resolve_dispute_cannot_be_called_twice() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.raise_dispute(&escrow_id, &client_addr));
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullRefund));

    // Try to resolve again
    super::assert_contract_error(
        client.try_resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullPayout),
        EscrowError::InvalidStatusTransition,
    );
}

/// A non-arbiter address cannot resolve a dispute.
#[test]
fn pause_blocks_raise_dispute() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, _, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.pause());

    super::assert_contract_error(
        client.try_resolve_dispute(&contract_id, &client_addr, &DisputeResolution::FullRefund),
        Error::UnauthorizedRole,
    );
    // Random outsider is also rejected.
    super::assert_contract_error(
        client.try_resolve_dispute(&contract_id, &outsider, &DisputeResolution::FullRefund),
        Error::UnauthorizedRole,
    );
    // Contract remains in Disputed state.
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );
}

/// After resolving a dispute, a second resolve is rejected (double-resolve).
#[test]
fn pause_blocks_resolve_dispute() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    // First resolution succeeds.
    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund,));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Refunded
    );

    // Second resolution must fail — contract is no longer Disputed.
    super::assert_contract_error(
        client.try_resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullPayout),
        Error::InvalidStatusTransition,
    );
}

/// After all milestones are released (settled), raise_dispute is rejected
/// because the contract is in Completed state, not Funded/PartiallyFunded.
#[test]
fn emergency_blocks_raise_and_resolve_dispute() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    assert!(client.activate_emergency_pause());

    super::assert_contract_error(
        client.try_raise_dispute(&contract_id, &client_addr),
        Error::InvalidState,
    );
}

/// Resolving a dispute moves funds: FullPayout sets released_amount and marks
/// Completed; FullRefund sets refunded_amount and marks Refunded. Verify that
/// the accounting is correct after each resolution type.
#[test]
fn resolve_moves_funds_correctly_full_payout() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _freelancer_addr, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullPayout,));
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(contract.released_amount, 100);
    assert_eq!(contract.refunded_amount, 0);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

#[test]
fn dispute_accounting_invariants_hold() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) = create_funded_contract_with_arbiter(
        &env,
        &client,
        vec![&env, 50_i128, 30_i128, 20_i128],
        100_i128,
    );

    // Release one milestone
    assert!(client.approve_milestone_release(&escrow_id, &client_addr, &0));
    assert!(client.release_milestone(&escrow_id, &client_addr, &0));

    let before_dispute = client.get_contract(&escrow_id);
    assert_eq!(before_dispute.released_amount, 50);
    assert_eq!(before_dispute.refunded_amount, 0);

    // Raise dispute
    assert!(client.raise_dispute(&escrow_id, &client_addr));

    // Resolve with split: remaining 50 → 20 refund, 30 release
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::Split(20, 30)));

    let after_dispute = client.get_contract(&escrow_id);
    assert_eq!(after_dispute.released_amount, 80); // 50 + 30
    assert_eq!(after_dispute.refunded_amount, 20);
    assert_eq!(after_dispute.total_deposited, 100);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

/// Raising a dispute on a contract in the Refunded (terminal) state is rejected.
#[test]
fn multiple_disputes_on_different_contracts() {
    let (env, _contract_id, client) = setup_initialized();

    // Create two contracts
    let (client1, _, arbiter1, escrow1) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    let (client2, _, arbiter2, escrow2) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 200_i128], 200_i128);

    // Raise and resolve disputes independently
    assert!(client.raise_dispute(&escrow1, &client1));
    assert!(client.raise_dispute(&escrow2, &client2));

    assert!(client.resolve_dispute(&escrow1, &arbiter1, &DisputeResolution::FullRefund));
    assert!(client.resolve_dispute(&escrow2, &arbiter2, &DisputeResolution::FullPayout));

    let contract1 = client.get_contract(&escrow1);
    let contract2 = client.get_contract(&escrow2);

    assert_eq!(contract1.status, ContractStatus::Refunded);
    assert_eq!(contract1.refunded_amount, 100);

    assert_eq!(contract2.status, ContractStatus::Completed);
    assert_eq!(contract2.released_amount, 200);
}

/// Resolving after the contract has been finalized is rejected with AlreadyFinalized.
#[test]
fn dispute_events_are_emitted() {
    let (env, _contract_id, client) = setup_initialized();
    let (client_addr, _, arbiter_addr, escrow_id) =
        create_funded_contract_with_arbiter(&env, &client, vec![&env, 100_i128], 100_i128);

    // Raise dispute
    assert!(client.raise_dispute(&escrow_id, &client_addr));

    // Check for dispute opened event
    let events = env.events().all();
    let dispute_opened = events.iter().any(|e| {
        if let Some((topics, _data)) = e.try_into() {
            topics
                == (
                    soroban_sdk::symbol_short!("dispute"),
                    soroban_sdk::symbol_short!("opened"),
                )
        } else {
            false
        }
    });
    assert!(dispute_opened, "dispute opened event not found");

    // Resolve dispute
    assert!(client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullRefund));

    // Check for dispute resolved event
    let events = env.events().all();
    let dispute_resolved = events.iter().any(|e| {
        if let Some((topics, _data)) = e.try_into() {
            topics
                == (
                    soroban_sdk::symbol_short!("dispute"),
                    soroban_sdk::symbol_short!("resolved"),
                )
        } else {
            false
        }
    });
    assert!(dispute_resolved, "dispute resolved event not found");
}
