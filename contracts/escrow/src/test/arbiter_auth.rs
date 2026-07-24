//! Authorization tests for `assign_arbiter` and `reassign_arbiter`.
//!
//! Covers who may set or change the arbiter, state gates for reassignment,
//! typed error codes on rejection, and the same-arbiter no-op path.

#![cfg(test)]

use crate::{
    ContractStatus, DisputeResolution, Error, Escrow, EscrowClient, ReleaseAuthorization,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

use super::assert_contract_error;

fn make_env() -> Env {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    env
}

fn make_client(env: &Env) -> EscrowClient<'_> {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    client
}

/// Create an unfunded contract without an arbiter.
fn contract_without_arbiter(
    env: &Env,
    client: &EscrowClient<'_>,
) -> (Address, Address, u32) {
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
    (client_addr, freelancer_addr, contract_id)
}

/// Create a funded contract without an arbiter.
fn funded_without_arbiter(
    env: &Env,
    client: &EscrowClient<'_>,
) -> (Address, Address, u32) {
    let (client_addr, freelancer_addr, contract_id) =
        contract_without_arbiter(env, client);
    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
    (client_addr, freelancer_addr, contract_id)
}

/// Create a funded contract with an arbiter already assigned at creation.
fn funded_with_arbiter(
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

// ---------------------------------------------------------------------------
// assign_arbiter — success paths
// ---------------------------------------------------------------------------

#[test]
fn client_can_assign_arbiter_on_created_contract() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, contract_id) = contract_without_arbiter(&env, &client);
    let arbiter_addr = Address::generate(&env);

    assert!(client.assign_arbiter(&contract_id, &client_addr, &arbiter_addr));
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(arbiter_addr)
    );
}

#[test]
fn freelancer_can_assign_arbiter_on_funded_contract() {
    let env = make_env();
    let client = make_client(&env);
    let (_, freelancer_addr, contract_id) = funded_without_arbiter(&env, &client);
    let arbiter_addr = Address::generate(&env);

    assert!(client.assign_arbiter(&contract_id, &freelancer_addr, &arbiter_addr));
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(arbiter_addr)
    );
}

#[test]
fn assign_arbiter_unblocks_dispute_after_assignment() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, contract_id) = funded_without_arbiter(&env, &client);
    let arbiter_addr = Address::generate(&env);

    assert!(client.assign_arbiter(&contract_id, &client_addr, &arbiter_addr));
    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );
}

// ---------------------------------------------------------------------------
// assign_arbiter — authorization and validation rejections
// ---------------------------------------------------------------------------

#[test]
fn assign_arbiter_by_non_party_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, contract_id) = contract_without_arbiter(&env, &client);
    let outsider = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);

    assert_contract_error(
        client.try_assign_arbiter(&contract_id, &outsider, &arbiter_addr),
        Error::UnauthorizedRole,
    );
    assert_eq!(client.get_contract(&contract_id).arbiter, None);
}

#[test]
fn assign_arbiter_when_already_set_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, arbiter_addr, contract_id) = funded_with_arbiter(&env, &client);
    let replacement = Address::generate(&env);

    assert_contract_error(
        client.try_assign_arbiter(&contract_id, &client_addr, &replacement),
        Error::InvalidState,
    );
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(arbiter_addr)
    );
}

#[test]
fn assign_arbiter_rejects_arbiter_equal_to_client() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, contract_id) = contract_without_arbiter(&env, &client);

    assert_contract_error(
        client.try_assign_arbiter(&contract_id, &client_addr, &client_addr),
        Error::InvalidArbiter,
    );
}

#[test]
fn assign_arbiter_rejects_arbiter_equal_to_freelancer() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, freelancer_addr, contract_id) =
        contract_without_arbiter(&env, &client);

    assert_contract_error(
        client.try_assign_arbiter(&contract_id, &client_addr, &freelancer_addr),
        Error::InvalidArbiter,
    );
}

#[test]
fn assign_arbiter_rejects_disputed_contract() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, arbiter_addr, contract_id) = funded_with_arbiter(&env, &client);
    assert!(client.raise_dispute(&contract_id, &client_addr));
    let new_arbiter = Address::generate(&env);

    assert_contract_error(
        client.try_assign_arbiter(&contract_id, &client_addr, &new_arbiter),
        Error::InvalidState,
    );
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(arbiter_addr)
    );
}

// ---------------------------------------------------------------------------
// reassign_arbiter — success paths
// ---------------------------------------------------------------------------

#[test]
fn client_can_reassign_arbiter_on_funded_contract() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, _old_arbiter, contract_id) = funded_with_arbiter(&env, &client);
    let new_arbiter = Address::generate(&env);

    assert!(client.reassign_arbiter(&contract_id, &client_addr, &new_arbiter));
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(new_arbiter)
    );
}

#[test]
fn freelancer_can_reassign_arbiter_on_funded_contract() {
    let env = make_env();
    let client = make_client(&env);
    let (_, freelancer_addr, _old_arbiter, contract_id) = funded_with_arbiter(&env, &client);
    let new_arbiter = Address::generate(&env);

    assert!(client.reassign_arbiter(&contract_id, &freelancer_addr, &new_arbiter));
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(new_arbiter)
    );
}

#[test]
fn reassign_arbiter_same_address_is_no_op() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, arbiter_addr, contract_id) = funded_with_arbiter(&env, &client);

    assert!(client.reassign_arbiter(&contract_id, &client_addr, &arbiter_addr));
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(arbiter_addr)
    );
}

// ---------------------------------------------------------------------------
// reassign_arbiter — authorization and validation rejections
// ---------------------------------------------------------------------------

#[test]
fn reassign_arbiter_by_non_party_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (_, _, _, contract_id) = funded_with_arbiter(&env, &client);
    let outsider = Address::generate(&env);
    let new_arbiter = Address::generate(&env);

    assert_contract_error(
        client.try_reassign_arbiter(&contract_id, &outsider, &new_arbiter),
        Error::UnauthorizedRole,
    );
}

#[test]
fn reassign_arbiter_when_none_assigned_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, contract_id) = funded_without_arbiter(&env, &client);
    let new_arbiter = Address::generate(&env);

    assert_contract_error(
        client.try_reassign_arbiter(&contract_id, &client_addr, &new_arbiter),
        Error::InvalidState,
    );
}

#[test]
fn reassign_arbiter_rejects_new_arbiter_equal_to_client() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, _, contract_id) = funded_with_arbiter(&env, &client);

    assert_contract_error(
        client.try_reassign_arbiter(&contract_id, &client_addr, &client_addr),
        Error::InvalidArbiter,
    );
}

#[test]
fn reassign_arbiter_rejects_new_arbiter_equal_to_freelancer() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, freelancer_addr, _, contract_id) = funded_with_arbiter(&env, &client);

    assert_contract_error(
        client.try_reassign_arbiter(&contract_id, &client_addr, &freelancer_addr),
        Error::InvalidArbiter,
    );
}

#[test]
fn reassign_arbiter_rejects_disputed_contract() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, old_arbiter, contract_id) = funded_with_arbiter(&env, &client);
    assert!(client.raise_dispute(&contract_id, &client_addr));
    let new_arbiter = Address::generate(&env);

    assert_contract_error(
        client.try_reassign_arbiter(&contract_id, &client_addr, &new_arbiter),
        Error::InvalidState,
    );
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(old_arbiter)
    );
}

#[test]
fn reassign_arbiter_after_dispute_resolved_is_rejected() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, arbiter_addr, contract_id) = funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullRefund,
    ));
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Refunded
    );

    let new_arbiter = Address::generate(&env);
    assert_contract_error(
        client.try_reassign_arbiter(&contract_id, &client_addr, &new_arbiter),
        Error::InvalidState,
    );
    assert_eq!(
        client.get_contract(&contract_id).arbiter,
        Some(arbiter_addr)
    );
}

#[test]
fn assign_arbiter_blocked_when_paused() {
    let env = make_env();
    let client = make_client(&env);
    let (client_addr, _, contract_id) = contract_without_arbiter(&env, &client);
    let arbiter_addr = Address::generate(&env);

    client.pause();
    assert_contract_error(
        client.try_assign_arbiter(&contract_id, &client_addr, &arbiter_addr),
        Error::ContractPaused,
    );
}
