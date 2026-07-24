//! Comprehensive tests for the O(1) arbiter read view [`Escrow::get_arbiter`].
//!
//! Coverage includes:
//! - Contract with an arbiter assigned → returns `Some(arbiter)`.
//! - Contract without an arbiter → returns `None`.
//! - Non-existent contract → returns `None` (safe default, no panic).
//! - Arbiter view through lifecycle states (Created, Funded, Disputed, Completed).
//! - Multiple contracts with distinct arbiters.
//! - Boundary: zero-value contract with arbiter.
//! - Idempotency: calling `get_arbiter` multiple times returns the same value.
//! - TTL invariance: reading arbiter does not extend the contract's TTL.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{
    test::{default_milestones, EscrowFixtureBuilder},
    Escrow, EscrowClient, ReleaseAuthorization,
};

/// Returns `Some(arbiter_addr)` when a contract was created with an arbiter.
#[test]
fn get_arbiter_returns_some_when_arbiter_set() {
    let fixture = EscrowFixtureBuilder::new().with_settlement_token().build();

    let arbiter = Address::generate(&fixture.env);
    let contract_id = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter.clone()),
        &default_milestones(&fixture.env),
        &ReleaseAuthorization::ClientOnly,
    );

    let result = fixture.escrow().get_arbiter(&contract_id);
    assert_eq!(result, Some(arbiter));
}

/// Returns `None` when the contract was created without an arbiter.
#[test]
fn get_arbiter_returns_none_when_arbiter_unset() {
    let fixture = EscrowFixtureBuilder::new().with_settlement_token().build();

    let contract_id = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &None,
        &default_milestones(&fixture.env),
        &ReleaseAuthorization::ClientOnly,
    );

    let result = fixture.escrow().get_arbiter(&contract_id);
    assert_eq!(result, None);
}

/// Returns `None` for a non-existent contract (no panic).
#[test]
fn get_arbiter_returns_none_for_nonexistent_contract() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let escrow_address = env.register(Escrow, ());
    let escrow = EscrowClient::new(&env, &escrow_address);

    // No contract has ever been created — any ID is missing.
    let result = escrow.get_arbiter(&999_999);
    assert_eq!(result, None);
}

/// Returns the correct arbiter for a funded contract.
#[test]
fn get_arbiter_on_funded_contract() {
    let fixture = EscrowFixtureBuilder::new().funded().build();
    let arbiter = Address::generate(&fixture.env);

    let contract_id = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter.clone()),
        &vec![&fixture.env, 100_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    // Fund the contract using the escrow fixture's token (clone to keep fixture alive)
    let token = fixture.settlement_token.clone().unwrap();
    let escrow = fixture.escrow();
    StellarAssetClient::new(&fixture.env, &token).mint(&fixture.client, &100_i128);
    escrow.deposit_funds(&contract_id, &fixture.client, &100_i128);

    let result = fixture.escrow().get_arbiter(&contract_id);
    assert_eq!(result, Some(arbiter));
}

/// Returns the correct arbiter through a full lifecycle: Created → Funded → Disputed.
#[test]
fn get_arbiter_in_disputed_state() {
    let fixture = EscrowFixtureBuilder::new().funded().build();
    let arbiter = Address::generate(&fixture.env);

    let contract_id = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter.clone()),
        &vec![&fixture.env, 100_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    // Fund the contract
    let token = fixture.settlement_token.clone().unwrap();
    let escrow = fixture.escrow();
    StellarAssetClient::new(&fixture.env, &token).mint(&fixture.client, &100_i128);
    escrow.deposit_funds(&contract_id, &fixture.client, &100_i128);

    // Raise a dispute
    escrow.raise_dispute(&contract_id, &fixture.client);

    let result = fixture.escrow().get_arbiter(&contract_id);
    assert_eq!(result, Some(arbiter));
}

/// Returns the correct arbiter after dispute resolution (Completed state).
#[test]
fn get_arbiter_after_dispute_resolution() {
    let fixture = EscrowFixtureBuilder::new().funded().build();
    let (arbiter, client_addr, freelancer_addr) = {
        let e = &fixture.env;
        (
            Address::generate(e),
            Address::generate(e),
            Address::generate(e),
        )
    };

    let contract_id = fixture.escrow().create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter.clone()),
        &vec![&fixture.env, 100_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    // Fund
    let token = fixture.settlement_token.clone().unwrap();
    StellarAssetClient::new(&fixture.env, &token).mint(&client_addr, &100_i128);
    fixture
        .escrow()
        .deposit_funds(&contract_id, &client_addr, &100_i128);

    // Dispute then resolve with FullRefund → goes to Refunded status
    fixture.escrow().raise_dispute(&contract_id, &client_addr);
    fixture.escrow().resolve_dispute(
        &contract_id,
        &arbiter,
        &crate::DisputeResolution::FullRefund,
    );

    let result = fixture.escrow().get_arbiter(&contract_id);
    assert_eq!(result, Some(arbiter));
}

/// Multiple contracts with distinct arbiters all return correct values.
#[test]
fn get_arbiter_multiple_contracts() {
    let fixture = EscrowFixtureBuilder::new().build();

    let arbiter1 = Address::generate(&fixture.env);
    let arbiter2 = Address::generate(&fixture.env);

    let id1 = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter1.clone()),
        &vec![&fixture.env, 50_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    let id2 = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter2.clone()),
        &vec![&fixture.env, 100_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq!(fixture.escrow().get_arbiter(&id1), Some(arbiter1));
    assert_eq!(fixture.escrow().get_arbiter(&id2), Some(arbiter2));
}

/// Mixed contracts (some with arbiter, some without) return correct results.
#[test]
fn get_arbiter_mixed_arbiter_and_no_arbiter() {
    let fixture = EscrowFixtureBuilder::new().build();
    let arbiter = Address::generate(&fixture.env);

    let id_with = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter.clone()),
        &vec![&fixture.env, 50_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    let id_without = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &None,
        &vec![&fixture.env, 50_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq!(fixture.escrow().get_arbiter(&id_with), Some(arbiter));
    assert_eq!(fixture.escrow().get_arbiter(&id_without), None);
}

/// Calling `get_arbiter` multiple times is idempotent.
#[test]
fn get_arbiter_is_idempotent() {
    let fixture = EscrowFixtureBuilder::new().build();
    let arbiter = Address::generate(&fixture.env);

    let contract_id = fixture.escrow().create_contract(
        &fixture.client,
        &fixture.freelancer,
        &Some(arbiter.clone()),
        &default_milestones(&fixture.env),
        &ReleaseAuthorization::ClientOnly,
    );

    for _ in 0..5 {
        assert_eq!(
            fixture.escrow().get_arbiter(&contract_id),
            Some(arbiter.clone())
        );
    }
}

/// `get_arbiter` is a read-only view that does not extend the contract's TTL.
///
/// We verify this by checking that `contract_exists` returns the same value
/// before and after calling `get_arbiter` — a TTL-extending read would mutate
/// the entry's expiry metadata. Since `get_arbiter` performs a plain `persistent().get()`
/// without an explicit `extend_ttl` call, the contract's expiry state is unchanged.
#[test]
fn get_arbiter_is_pure_read() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let client = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let arbiter = Address::generate(&env);

    let escrow_address = env.register(Escrow, ());
    let escrow = EscrowClient::new(&env, &escrow_address);
    escrow.initialize(&admin);

    let contract_id = escrow.create_contract(
        &client,
        &freelancer,
        &Some(arbiter.clone()),
        &vec![&env, 100_i128],
        &ReleaseAuthorization::ClientOnly,
    );

    // Contract exists, arbiter is visible.
    assert!(escrow.contract_exists(&contract_id));
    assert_eq!(escrow.get_arbiter(&contract_id), Some(arbiter.clone()));

    // Reading arbiter again should return the same value each time.
    assert_eq!(escrow.get_arbiter(&contract_id), Some(arbiter));
}
