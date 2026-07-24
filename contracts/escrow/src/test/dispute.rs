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
    Contract, ContractStatus, DisputeResolution, DisputeSplit, Error, Escrow, EscrowClient,
    ReleaseAuthorization,
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
    let milestones = vec![env, 100_i128];
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
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

// ---------------------------------------------------------------------------
// Integration tests: resolution variants
// ---------------------------------------------------------------------------

/// Integration: FullRefund on a funded contract conserves balance and marks Refunded.
#[test]
fn resolve_full_refund_conserves_and_marks_refunded() {
    let env = make_env();
    let client = make_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let milestones = soroban_sdk::vec![&env, 125_i128, 75_i128];
    let escrow_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    client.deposit_funds(&escrow_id, &client_addr, &200_i128);

    client.raise_dispute(&escrow_id, &client_addr);
    client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullRefund);

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
fn resolve_full_payout_conserves_and_marks_completed() {
    let env = make_env();
    let client = make_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let milestones = soroban_sdk::vec![&env, 150_i128];
    let escrow_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    client.deposit_funds(&escrow_id, &client_addr, &150_i128);

    client.raise_dispute(&escrow_id, &client_addr);
    client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::FullPayout);

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
fn resolve_partial_refund_conserves_70_30_split() {
    let env = make_env();
    let client = make_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let milestones = soroban_sdk::vec![&env, 100_i128];
    let escrow_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    client.deposit_funds(&escrow_id, &client_addr, &100_i128);

    client.raise_dispute(&escrow_id, &client_addr);
    client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::PartialRefund);

    let contract = client.get_contract(&escrow_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

/// Integration: Split accepts valid custom amounts and conserves balance.
#[test]
fn resolve_split_conserves_custom_amounts() {
    let env = make_env();
    let client = make_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let milestones = soroban_sdk::vec![&env, 40_i128, 60_i128];
    let escrow_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    client.deposit_funds(&escrow_id, &client_addr, &100_i128);

    client.raise_dispute(&escrow_id, &client_addr);
    let split = DisputeSplit {
        client_amount: 35,
        freelancer_amount: 65,
    };
    client.resolve_dispute(&escrow_id, &arbiter_addr, &DisputeResolution::Split(split));

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
fn raise_dispute_by_client_succeeds() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, _, contract_id) = funded_contract_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );
}

/// Freelancer can also raise a dispute on a funded contract with an arbiter.
#[test]
fn raise_dispute_by_freelancer_succeeds() {
    let env = make_env();
    let client = make_client(&env);
    let (_, freelancer_addr, _, contract_id) = funded_contract_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &freelancer_addr));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );
}

/// A non-party (random address) cannot raise a dispute.
#[test]
fn raise_dispute_by_non_party_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, _, contract_id) = funded_contract_with_arbiter(&env, &client);
    let outsider = Address::generate(&env);

    super::assert_contract_error(
        client.try_raise_dispute(&contract_id, &outsider),
        Error::UnauthorizedRole,
    );
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Funded
    );
}

/// Raising a dispute on a contract without an arbiter is rejected.
#[test]
fn raise_dispute_without_arbiter_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, contract_id) = funded_contract_no_arbiter(&env, &client);

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
fn raise_dispute_on_completed_contract_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let milestones = vec![&env, 100_i128];
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
    // Release the only milestone to reach Completed state.
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Completed
    );

    super::assert_contract_error(
        client.try_raise_dispute(&contract_id, &client_addr),
        Error::InvalidState,
    );
}

/// The designated arbiter can resolve a dispute.
#[test]
fn resolve_dispute_by_arbiter_succeeds() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund,));
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Refunded);
    assert_eq!(contract.refunded_amount, 100);
}

/// A non-arbiter address cannot resolve a dispute.
#[test]
fn resolve_dispute_by_non_arbiter_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, _, contract_id) = disputed_contract(&env, &client);
    let outsider = Address::generate(&env);

    // Client is a party but not the arbiter — must be rejected.
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
fn double_resolve_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

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
fn raise_dispute_after_settle_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let milestones = vec![&env, 50_i128, 50_i128];
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
    // Release all milestones to settle the contract.
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &1));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Completed
    );

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
fn resolve_moves_funds_correctly_full_refund() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund,));
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Refunded);
    assert_eq!(contract.released_amount, 0);
    assert_eq!(contract.refunded_amount, 100);
    assert_eq!(
        contract.released_amount + contract.refunded_amount,
        contract.funded_amount
    );
}

/// Raising a dispute on a contract in the Refunded (terminal) state is rejected.
#[test]
fn raise_dispute_on_refunded_contract_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr, contract_id) =
        funded_contract_with_arbiter(&env, &client);

    // Resolve as FullRefund to reach Refunded terminal state.
    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund,));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Refunded
    );

    // Cannot raise again.
    super::assert_contract_error(
        client.try_raise_dispute(&contract_id, &freelancer_addr),
        Error::AlreadyFinalized,
    );
}

/// Resolving after the contract has been finalized is rejected with AlreadyFinalized.
#[test]
fn resolve_after_finalize_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    // Finalize the disputed contract.
    assert!(client.finalize_contract(&contract_id, &client_addr));

    super::assert_contract_error(
        client.try_resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund),
        Error::AlreadyFinalized,
    );
}

// ---------------------------------------------------------------------------
// Dispute-record view (issue #795)
// ---------------------------------------------------------------------------

/// Unknown contract ids return `None` from `get_dispute_record` and `false`
/// from `has_dispute` — neither entrypoint panics for missing IDs, matching
/// the "sensible default" requirement of the indexer surface. This single
/// test exercises both read views on a range of never-allocated ids so that
/// future changes to the storage-key schema are caught here.
#[test]
fn dispute_view_returns_default_for_unknown_and_undisputed_contracts() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, _, contract_id) = funded_contract_with_arbiter(&env, &client);

    // Far above `get_next_contract_id` — these ids have never been allocated,
    // so neither read view should see anything.
    for id in [0_u32, 1, 42, 9999, u32::MAX] {
        assert_eq!(client.get_dispute_record(&id), None, "id {}", id);
        assert!(!client.has_dispute(&id), "id {}", id);
    }

    // An allocated-but-never-disputed contract also returns the default.
    assert_eq!(client.get_dispute_record(&contract_id), None);
    assert!(!client.has_dispute(&contract_id));
}

/// Raising a dispute records the raiser, raised_at_ledger, raised_at_timestamp,
/// and leaves the resolution side as None.
#[test]
fn get_dispute_record_after_raise_keeps_resolution_side_none() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, _, contract_id) = funded_contract_with_arbiter(&env, &client);

    env.ledger().with_mut(|li| {
        li.sequence_number = 100;
        li.timestamp = 1_700_000_000;
    });

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.has_dispute(&contract_id));

    let record = client
        .get_dispute_record(&contract_id)
        .expect("record should exist after raise");
    assert_eq!(record.raiser, client_addr);
    assert_eq!(record.raised_at_ledger, 100);
    assert_eq!(record.raised_at_timestamp, 1_700_000_000);
    assert!(record.resolver.is_none());
    assert!(record.resolution.is_none());
    assert!(record.resolved_at_ledger.is_none());
    assert!(record.resolved_at_timestamp.is_none());
}

/// Freelancer-initiated disputes store the freelancer address as the raiser.
#[test]
fn get_dispute_record_after_raise_records_freelancer_as_raiser() {
    let env = make_env();
    let client = make_client(&env);
    let (_, freelancer_addr, _, contract_id) = funded_contract_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &freelancer_addr));
    let record = client
        .get_dispute_record(&contract_id)
        .expect("record should exist after raise");
    assert_eq!(record.raiser, freelancer_addr);
    assert!(record.resolution.is_none());
}

/// After a FullPayout resolve, the record carries the arbiter, FullPayout
/// resolution, and the resolved-side timestamps.
#[test]
fn get_dispute_record_after_resolve_full_payout_populates_resolution_side() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    env.ledger().with_mut(|li| {
        li.sequence_number = 200;
        li.timestamp = 1_700_000_500;
    });

    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullPayout));

    assert!(client.has_dispute(&contract_id));
    let record = client
        .get_dispute_record(&contract_id)
        .expect("record should persist after resolve");
    assert_eq!(record.resolver, Some(arbiter_addr));
    assert_eq!(record.resolution, Some(DisputeResolution::FullPayout));
    assert_eq!(record.resolved_at_ledger, Some(200));
    assert_eq!(record.resolved_at_timestamp, Some(1_700_000_500));
}

/// After a FullRefund resolve, the record carries FullRefund as the resolution.
#[test]
fn get_dispute_record_after_resolve_full_refund_carries_variant() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullRefund));

    let record = client
        .get_dispute_record(&contract_id)
        .expect("record should persist after resolve");
    assert_eq!(record.resolver, Some(arbiter_addr));
    assert_eq!(record.resolution, Some(DisputeResolution::FullRefund));
}

/// After a PartialRefund resolve, the record still stores the raise-side
/// metadata and gains the PartialRefund resolution record on the resolve side.
#[test]
fn get_dispute_record_after_resolve_partial_refund_preserves_raise_side() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr, contract_id) =
        funded_contract_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    let before = client
        .get_dispute_record(&contract_id)
        .expect("record exists after raise");
    assert_eq!(before.raiser, client_addr);
    assert!(before.resolution.is_none());

    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::PartialRefund));
    let after = client
        .get_dispute_record(&contract_id)
        .expect("record persists after resolve");
    assert_eq!(after.raiser, client_addr);
    assert_eq!(after.raised_at_ledger, before.raised_at_ledger);
    assert_eq!(after.raised_at_timestamp, before.raised_at_timestamp);
    assert_eq!(after.resolver, Some(arbiter_addr));
    assert_eq!(after.resolution, Some(DisputeResolution::PartialRefund));
    assert!(after.resolved_at_ledger.is_some());
    assert!(after.resolved_at_timestamp.is_some());
}

/// After a Split resolve, both legs of the split are stored verbatim and
/// `get_dispute_record` is round-trippable.
#[test]
fn get_dispute_record_after_resolve_split_round_trips() {
    let env = make_env();
    let client = make_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let milestones = soroban_sdk::vec![&env, 100_i128];
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    client.deposit_funds(&contract_id, &client_addr, &100_i128);
    client.raise_dispute(&contract_id, &client_addr);

    let split = DisputeSplit {
        client_amount: 35,
        freelancer_amount: 65,
    };
    client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::Split(split.clone()));

    let record = client.get_dispute_record(&contract_id).unwrap();
    match record.resolution.expect("split resolution stored") {
        DisputeResolution::Split(s) => {
            assert_eq!(s.client_amount, 35);
            assert_eq!(s.freelancer_amount, 65);
        }
        other => panic!("expected Split resolution, got {:?}", other),
    }
    assert_eq!(record.resolver, Some(arbiter_addr));
    assert_eq!(record.raiser, client_addr);
    let _ = freelancer_addr; // silence unused warning when arbiter is named
}

/// `has_dispute` remains true after resolve and only flips to false when the
/// underlying storage entry is evicted or removed. Indexers can therefore
/// rely on it as the cheap pre-flight probe for "dispute seen" regardless
/// of whether the dispute has been resolved.
#[test]
fn has_dispute_remains_true_after_resolve() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, arbiter_addr, contract_id) = disputed_contract(&env, &client);

    assert!(client.has_dispute(&contract_id));
    assert!(client.resolve_dispute(&contract_id, &arbiter_addr, &DisputeResolution::FullPayout));
    assert!(
        client.has_dispute(&contract_id),
        "record persists after resolve for audit purposes"
    );
}

/// Reading `get_dispute_record` does not panic on unknown contract ids and
/// returns `Some(record)` if the dispute was ever raised — even if the
/// contract has since been resolved to `Completed` or `Refunded`.
#[test]
fn get_dispute_record_does_not_panic_on_unknown_contract_id() {
    let env = make_env();
    let client = make_client(&env);

    // `u32::MAX` is far above the high-water mark — no contract was ever
    // assigned this id, so the read view must return `None` and `false`
    // rather than panicking.
    assert_eq!(client.get_dispute_record(&u32::MAX), None);
    assert!(!client.has_dispute(&u32::MAX));
}

/// A non-arbiter resolve attempt must panic with `UnauthorizedRole` AND leave
/// the dispute record in its original raise-side state (fail-closed).
/// Covers the reviewer's "non-arbiter resolve leaves record untouched" gap.
#[test]
fn non_arbiter_resolve_leaves_dispute_record_unchanged() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr, contract_id) =
        funded_contract_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    let before = client
        .get_dispute_record(&contract_id)
        .expect("record exists after raise");

    // Both the client and an outsider are rejected — neither has arbiter auth.
    let outsider = Address::generate(&env);
    for rejected in [client_addr.clone(), freelancer_addr.clone(), outsider] {
        super::assert_contract_error(
            client.try_resolve_dispute(&contract_id, &rejected, &DisputeResolution::FullPayout),
            Error::UnauthorizedRole,
        );
    }

    // The record must still reflect only the raise side after every rejection.
    assert!(client.has_dispute(&contract_id));
    let after = client
        .get_dispute_record(&contract_id)
        .expect("record survives rejected resolve");
    assert_eq!(after.raiser, before.raiser);
    assert_eq!(after.raised_at_ledger, before.raised_at_ledger);
    assert_eq!(after.raised_at_timestamp, before.raised_at_timestamp);
    assert!(after.resolver.is_none());
    assert!(after.resolution.is_none());
    assert!(after.resolved_at_ledger.is_none());
    assert!(after.resolved_at_timestamp.is_none());
}
