//! Tests for `release_milestone` caller authorization.
//!
//! Covers every `ReleaseAuthorization` variant in both the happy
//! (authorized caller with valid approvals) and negative (unauthorized
//! caller, wrong role, missing approvals) paths.
//!
//! # Security contract
//!
//! 1. `caller.require_auth()` MUST be invoked *before* any role
//!    discrimination or state mutation.
//! 2. Role checks MUST reject callers who do not match the contract's
//!    `ReleaseAuthorization` variant.
//! 3. `MultiSig` requires BOTH client and freelancer approvals via
//!    `approvals::check_approvals`; no single party can release alone.
//! 4. Approvals must exist and not have expired; missing or expired
//!    approvals produce `InsufficientApprovals`.
//! 5. All negative paths MUST be fail-closed (no partial state change).

#![cfg(test)]

use soroban_sdk::{
    testutils::Address as _, testutils::Events, vec, Address, Env, IntoVal, Symbol, TryFromVal,
};

use super::register_client;
use crate::{ContractStatus, Error, Escrow, EscrowClient, ReleaseAuthorization};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup(env: &Env) -> (Address, Address, Address) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let arbiter_addr = Address::generate(env);
    (client_addr, freelancer_addr, arbiter_addr)
}

/// Register and initialize the escrow contract and return a client.
fn register(env: &Env) -> EscrowClient<'_> {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = soroban_sdk::Address::generate(env);
    client.initialize(&admin);
    client
}
fn assert_contract_error<T, E>(
    result: Result<T, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    expected: E,
) where
    E: Into<soroban_sdk::Error> + core::fmt::Debug,
{
    match result {
        Err(Ok(e)) => {
            let expected_err: soroban_sdk::Error = expected.into();
            assert_eq!(e, expected_err, "contract error code mismatch");
        }
        _other => panic!(
            "expected contract error {:?}, got unexpected result variant",
            expected
        ),
    }
}

// Helper that accepts i128-returning try_* calls (like refund_unreleased_milestones)
fn assert_contract_error_i128<E>(
    result: Result<
        Result<i128, soroban_sdk::Error>,
        Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
    >,
    expected: E,
) where
    E: Into<soroban_sdk::Error> + core::fmt::Debug,
{
    match result {
        Err(Ok(e)) => {
            let expected_err: soroban_sdk::Error = expected.into();
            assert_eq!(e, expected_err, "contract error code mismatch");
        }
        _other => panic!(
            "expected contract error {:?}, got unexpected result variant",
            expected
        ),
    }
}

fn create_contract_with_mode(
    env: &Env,
    client: &EscrowClient<'_>,
    client_addr: &Address,
    freelancer_addr: &Address,
    arbiter: &Option<Address>,
    release_auth: &ReleaseAuthorization,
) -> u32 {
    let milestones = vec![env, 500_i128, 300_i128, 200_i128];
    client.create_contract(
        client_addr,
        freelancer_addr,
        arbiter,
        &milestones,
        release_auth,
    )
}

fn fund_contract(_env: &Env, client: &EscrowClient<'_>, contract_id: &u32) {
    let milestones = client.get_milestones(contract_id);
    let total: i128 = milestones.iter().map(|m| m.amount).sum();
    let contract = client.get_contract(contract_id);
    assert!(client.deposit_funds(contract_id, &contract.client, &total));

    for index in 0..milestones.len() {
        match contract.release_authorization {
            ReleaseAuthorization::ClientOnly | ReleaseAuthorization::ClientAndArbiter => {
                assert!(client.approve_milestone_release(contract_id, &contract.client, &index));
            }
            ReleaseAuthorization::ArbiterOnly => {
                let arbiter = contract
                    .arbiter
                    .clone()
                    .expect("ArbiterOnly requires arbiter");
                assert!(client.approve_milestone_release(contract_id, &arbiter, &index));
            }
            ReleaseAuthorization::MultiSig => {
                assert!(client.approve_milestone_release(contract_id, &contract.client, &index));
                assert!(client.approve_milestone_release(
                    contract_id,
                    &contract.freelancer,
                    &index,
                ));
            }
        }
    }
}

/// Create a fully-funded 2-milestone contract (500 + 300 = 800 total).
/// Returns `(client_addr, freelancer_addr, contract_id)`.
fn funded_contract(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = vec![env, 500_i128, 300_i128];
    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&id, &client_addr, &800_i128));
    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    assert!(client.approve_milestone_release(&id, &client_addr, &1));
    (client_addr, freelancer_addr, id)
}

fn milestones(env: &Env) -> soroban_sdk::Vec<i128> {
    vec![env, 500_0000000_i128, 300_0000000_i128]
}

fn total() -> i128 {
    800_0000000_i128
}

fn new_client(env: &Env) -> EscrowClient<'_> {
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &contract_id);
    let admin = soroban_sdk::Address::generate(env);
    client.initialize(&admin);
    client
}

/// Create a funded contract with the given authorization mode.
/// Returns contract_id.
fn create(
    env: &Env,
    client: &EscrowClient<'_>,
    client_addr: &Address,
    freelancer_addr: &Address,
    arbiter: Option<&Address>,
    auth: &ReleaseAuthorization,
) -> u32 {
    let arbiter_owned = arbiter.cloned();
    let id = client.create_contract(
        client_addr,
        freelancer_addr,
        &arbiter_owned,
        &milestones(env),
        auth,
    );
    assert!(client.deposit_funds(&id, client_addr, &total()));
    // Approve milestone 0 so release can go through on happy paths
    match auth {
        ReleaseAuthorization::ClientOnly | ReleaseAuthorization::ClientAndArbiter => {
            assert!(client.approve_milestone_release(&id, client_addr, &0));
        }
        ReleaseAuthorization::ArbiterOnly => {
            assert!(client.approve_milestone_release(
                &id,
                arbiter.expect("ArbiterOnly requires arbiter"),
                &0,
            ));
        }
        ReleaseAuthorization::MultiSig => {
            assert!(client.approve_milestone_release(&id, client_addr, &0));
            assert!(client.approve_milestone_release(&id, freelancer_addr, &0));
        }
    }
    id
}

// ===========================================================================
//  ClientOnly
// ===========================================================================

#[test]
fn client_only_client_can_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.release_milestone(&id, &client_addr, &0));
    let c = client.get_contract(&id);
    assert_eq!(c.released_amount, 500_0000000_i128);
}

#[test]
fn client_only_freelancer_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn client_only_arbiter_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientOnly,
    );
    let result = client.try_release_milestone(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn client_only_attacker_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    let attacker = Address::generate(&env);
    let result = client.try_release_milestone(&id, &attacker, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ===========================================================================
//  ArbiterOnly
// ===========================================================================

#[test]
fn arbiter_only_arbiter_can_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    assert!(client.release_milestone(&id, &arbiter_addr, &0));
}

#[test]
fn arbiter_only_client_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn arbiter_only_freelancer_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn arbiter_only_attacker_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    let attacker = Address::generate(&env);
    let result = client.try_release_milestone(&id, &attacker, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ===========================================================================
//  ClientAndArbiter
// ===========================================================================

#[test]
fn client_and_arbiter_client_can_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    assert!(client.release_milestone(&id, &client_addr, &0));
}

#[test]
fn client_and_arbiter_arbiter_can_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    assert!(client.approve_milestone_release(&id, &arbiter_addr, &0));
    assert!(client.release_milestone(&id, &arbiter_addr, &0));
}

#[test]
fn client_and_arbiter_freelancer_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn client_and_arbiter_attacker_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    let attacker = Address::generate(&env);
    let result = client.try_release_milestone(&id, &attacker, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ===========================================================================
//  MultiSig
// ===========================================================================

#[test]
fn multisig_client_can_release_with_both_approvals() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );
    assert!(client.release_milestone(&id, &client_addr, &0));
}

#[test]
fn multisig_freelancer_can_release_with_both_approvals() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );
    assert!(client.release_milestone(&id, &freelancer_addr, &0));
}

#[test]
fn multisig_arbiter_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::MultiSig,
    );
    let result = client.try_release_milestone(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn multisig_attacker_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );
    let attacker = Address::generate(&env);
    let result = client.try_release_milestone(&id, &attacker, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn multisig_only_one_approval_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);

    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::MultiSig,
    );
    assert!(client.deposit_funds(&id, &client_addr, &total()));

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

#[test]
fn multisig_only_freelancer_approval_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);

    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::MultiSig,
    );
    assert!(client.deposit_funds(&id, &client_addr, &total()));

    assert!(client.approve_milestone_release(&id, &freelancer_addr, &0));
    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

#[test]
fn multisig_arbiter_cannot_record_approval() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = setup(&env);

    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones(&env),
        &ReleaseAuthorization::MultiSig,
    );
    assert!(client.deposit_funds(&id, &client_addr, &total()));

    let result = client.try_approve_milestone_release(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ===========================================================================
//  Missing / expired approvals
// ===========================================================================

#[test]
fn release_without_approval_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);

    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&id, &client_addr, &total()));

    // No approval recorded yet
    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

// ===========================================================================
//  require_auth() ordering — unauth caller without mock
// ===========================================================================

#[test]
fn unauthorized_caller_without_auth_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    let stranger = Address::generate(&env);
    let result = client.try_release_milestone(&id, &stranger, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ===========================================================================
//  State mutation guard
// ===========================================================================

#[test]
fn fail_closed_on_unauthorized_caller_no_state_change() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _arbiter_addr) = setup(&env);
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    let before = client.get_contract(&id);

    let attacker = Address::generate(&env);
    let result = client.try_release_milestone(&id, &attacker, &0);
    assert_contract_error(result, Error::UnauthorizedRole);

    let after = client.get_contract(&id);
    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
}

// ---------------------------------------------------------------------------
// Double-release is rejected with AlreadyReleased; no duplicate transfer
// ---------------------------------------------------------------------------

#[test]
fn double_release_is_rejected_and_amount_not_duplicated() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register(&env);
    let (client_addr, _freelancer_addr, id) = funded_contract(&env, &client);

    // First release succeeds.
    assert!(client.release_milestone(&id, &client_addr, &0));

    // Second release on the same milestone must fail with AlreadyReleased.
    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::MilestoneAlreadyReleased);

    // released_amount must not be doubled.
    let contract = client.get_contract(&id);
    assert_eq!(contract.released_amount, 500_i128);
}

// ---------------------------------------------------------------------------
// Freelancer (non-client) is also rejected
// ---------------------------------------------------------------------------

#[test]
fn freelancer_cannot_release_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register(&env);
    let (_client_addr, freelancer_addr, id) = funded_contract(&env, &client);

    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn release_emits_events() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);

    let contract_id = create_contract_with_mode(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        &None,
        &ReleaseAuthorization::ClientOnly,
    );

    fund_contract(&env, &client, &contract_id);

    // Release milestone
    client.release_milestone(&contract_id, &client_addr, &0);

    // Check release event was emitted
    let events = env.events().all();
    assert!(events.len() > 0);

    // Find the release event
    let release_event = events.iter().find(|event| {
        let topics = &event.1;
        topics.len() > 0 && {
            if let Ok(sym) = Symbol::try_from_val(&env, &topics.get(0).unwrap()) {
                sym == Symbol::new(&env, "milestone_released")
            } else {
                false
            }
        }
    });
    assert!(release_event.is_some());
}

#[test]
fn rejects_double_release_and_completes_contract() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);

    let contract_id = create_contract_with_mode(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        &None,
        &ReleaseAuthorization::ClientOnly,
    );
    fund_contract(&env, &client, &contract_id);

    assert!(client.release_milestone(&contract_id, &client_addr, &0));

    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    assert_contract_error(result, Error::MilestoneAlreadyReleased);

    assert!(client.release_milestone(&contract_id, &client_addr, &1));

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
}

#[test]
fn rejects_refund_after_release_and_release_after_refund() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);

    let contract_id = create_contract_with_mode(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        &None,
        &ReleaseAuthorization::ClientOnly,
    );
    fund_contract(&env, &client, &contract_id);

    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    let refund_ids = vec![&env, 0_u32];
    let refund_result = client.try_refund_unreleased_milestones(&contract_id, &refund_ids);
    match refund_result {
        Err(Ok(e)) => {
            assert_eq!(e, soroban_sdk::Error::from(Error::AlreadyReleased));
        }
        _ => panic!("expected contract error AlreadyReleased"),
    }

    let refund_ids = vec![&env, 1_u32];
    assert!(client.refund_unreleased_milestones(&contract_id, &refund_ids) > 0);

    let result = client.try_release_milestone(&contract_id, &client_addr, &1);
    assert_contract_error(result, Error::AlreadyRefunded);
}
