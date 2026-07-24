//! Boundary and rejection tests for the settlement logic.
//!
//! Covers edge cases: exactly-at boundary, one over, and unauthorized caller for
//! `deposit_funds` and `release_milestone` entrypoints.

#![cfg(test)]

use super::assert_contract_error;
use crate::{Error, Escrow, EscrowClient, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Register the escrow contract and an SAC, initialize escrow, bind settlement
/// token. Returns `(escrow_client, sac_address)`.
fn setup_bound(env: &Env) -> (EscrowClient<'_>, Address) {
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &contract_id);
    let admin = Address::generate(env);

    let sac = env.register_stellar_asset_contract(admin.clone());

    env.mock_all_auths_allowing_non_root_auth();
    client.initialize(&admin);
    client.bind_settlement_token(&admin, &sac);

    (client, sac)
}

/// Mint tokens to `holder`.
fn mint_to(env: &Env, sac: &Address, holder: &Address, amount: i128) {
    StellarAssetClient::new(env, sac).mint(holder, &amount);
}

/// Helper: creates a basic contract with 2 milestones: [50, 150].
/// Returns `(client_addr, freelancer_addr, contract_id)`.
fn setup_contract(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = vec![env, 50_i128, 150_i128]; // Total = 200

    env.mock_all_auths_allowing_non_root_auth();
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    (client_addr, freelancer_addr, contract_id)
}

// ─── deposit_funds boundaries ────────────────────────────────────────────────

#[test]
fn deposit_funds_accepts_exactly_at_boundary() {
    let env = Env::default();
    let (client, sac) = setup_bound(&env);
    let (client_addr, _, contract_id) = setup_contract(&env, &client);

    // Total required is 200. Deposit exactly 200 (exactly-at boundary).
    mint_to(&env, &sac, &client_addr, 200);

    env.mock_all_auths_allowing_non_root_auth();
    assert!(client.deposit_funds(&contract_id, &client_addr, &200_i128));
}

#[test]
fn deposit_funds_rejects_one_over() {
    let env = Env::default();
    let (client, sac) = setup_bound(&env);
    let (client_addr, _, contract_id) = setup_contract(&env, &client);

    // Total required is 200. We try to deposit 201 (one over).
    mint_to(&env, &sac, &client_addr, 300);

    env.mock_all_auths_allowing_non_root_auth();
    let result = client.try_deposit_funds(&contract_id, &client_addr, &201_i128);
    assert_contract_error(result, Error::InvalidDepositAmount);
}

#[test]
fn deposit_funds_rejects_unauthorized_caller() {
    let env = Env::default();
    let (client, sac) = setup_bound(&env);
    let (client_addr, _, contract_id) = setup_contract(&env, &client);

    let unauthorized = Address::generate(&env);
    mint_to(&env, &sac, &unauthorized, 100);

    env.mock_all_auths_allowing_non_root_auth();
    // Only `client_addr` can deposit. Even though `unauthorized` has tokens and could auth,
    // the contract rejects non-client depositors.
    let result = client.try_deposit_funds(&contract_id, &unauthorized, &100_i128);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ─── release_milestone boundaries ────────────────────────────────────────────

#[test]
fn release_milestone_accepts_exactly_at_boundary_index() {
    let env = Env::default();
    let (client, sac) = setup_bound(&env);
    let (client_addr, _, contract_id) = setup_contract(&env, &client);

    mint_to(&env, &sac, &client_addr, 200);
    env.mock_all_auths_allowing_non_root_auth();
    client.deposit_funds(&contract_id, &client_addr, &200_i128);

    // Milestones are at index 0 and 1. 1 is exactly at the upper bound.
    client.approve_milestone_release(&contract_id, &client_addr, &1);
    assert!(client.release_milestone(&contract_id, &client_addr, &1));
}

#[test]
fn release_milestone_rejects_one_over_index() {
    let env = Env::default();
    let (client, sac) = setup_bound(&env);
    let (client_addr, _, contract_id) = setup_contract(&env, &client);

    mint_to(&env, &sac, &client_addr, 200);
    env.mock_all_auths_allowing_non_root_auth();
    client.deposit_funds(&contract_id, &client_addr, &200_i128);

    // Milestones are length 2, max valid index is 1. We try index 2 (one over).
    let result = client.try_release_milestone(&contract_id, &client_addr, &2);
    assert_contract_error(result, Error::IndexOutOfBounds);
}

#[test]
fn release_milestone_rejects_unauthorized_caller() {
    let env = Env::default();
    let (client, sac) = setup_bound(&env);
    let (client_addr, _, contract_id) = setup_contract(&env, &client);

    mint_to(&env, &sac, &client_addr, 200);
    env.mock_all_auths_allowing_non_root_auth();
    client.deposit_funds(&contract_id, &client_addr, &200_i128);

    let unauthorized = Address::generate(&env);

    // Using `unauthorized` caller instead of `client_addr` for `ClientOnly` release.
    let result = client.try_release_milestone(&contract_id, &unauthorized, &0);
    assert_contract_error(result, crate::EscrowError::UnauthorizedRole);
}
