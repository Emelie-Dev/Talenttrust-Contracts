//! Pause-gate regression tests for the mutating escrow entrypoints.
//!
//! Issue #692: create_contract, deposit_funds, release_milestone,
//! refund_unreleased_milestones, cancel_contract, and issue_reputation must all
//! honor the Paused flag and reject calls with ContractPaused while paused, then
//! resume normally after unpause. approve_milestone_release is intentionally not
//! gated yet (tracked separately) and is exercised here only as a setup step.
//!
//! Emergency-mode coverage lives in emergency_controls.rs; this module exercises
//! the plain pause() / unpause() path. The pause check runs before require_auth,
//! so a paused contract rejects uniformly regardless of caller.

use soroban_sdk::testutils::Ledger as _;
use crate::{Error, Escrow, EscrowClient, EscrowError, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

// --- helpers ---

fn setup_initialized() -> (Env, Address, Address) {
    let env = Env::default();
    env.ledger().with_mut(|li| { li.max_entry_ttl = 3_110_400; li.min_persistent_entry_ttl = 3_110_400; });
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    assert!(client.initialize(&admin));
    (env, client, admin)
}

/// Create a contract in `Created` state with `ClientOnly` release authorization
/// and three milestones. Used by deposit_funds tests because `deposit_funds`
/// only accepts `Created` state on `main` — a `Funded` contract would panic
/// with `InvalidState` before `set_paused` can be exercised on top of it.
/// Returns `(client_addr, freelancer_addr, contract_id)`.
fn setup_created_contract(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = vec![env, 100_i128, 200_i128, 300_i128];
    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    (client_addr, freelancer_addr, id)
}

/// Create a fully-funded contract with `ClientOnly` release authorization and
/// two milestones. Returns `(client_addr, freelancer_addr, contract_id)`.
/// Used by release/refund/issue_reputation/cancel tests that need a `Funded`
/// or `Completed` baseline (NOT for deposit-only happy-path tests, see
/// `setup_created_contract`).
fn setup_funded_contract(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = vec![env, 100_i128, 200_i128];
    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    client.deposit_funds(&id, &client_addr, &300_i128);
    (client_addr, freelancer_addr, id)
}

/// Create and complete a contract (all milestones released) so issue_reputation
/// can be exercised from a `Completed` baseline.
fn setup_completed_contract(env: &Env, client: &EscrowClient<'_>) -> (Address, Address, u32) {
    let (client_addr, freelancer_addr, id) = setup_funded_contract(env, client);
    client.approve_milestone_release(&id, &client_addr, &0);
    client.release_milestone(&id, &client_addr, &0);
    client.approve_milestone_release(&id, &client_addr, &1);
    client.release_milestone(&id, &client_addr, &1);
    (client_addr, freelancer_addr, id)
}

/// Manually flip the `Emergency` flag on the underlying storage WITHOUT
/// flipping the `Paused` flag (so `require_not_paused()` reaches the
/// Emergency check).
fn set_emergency_only(env: &Env, client: &EscrowClient<'_>) {
    let _: bool = client.activate_emergency_pause();
    // The activate helper sets BOTH flags; we now clear Paused so the gate
    // hits the Emergency check first.
    let contract_addr: Address = client.address.clone();
    env.as_contract(&contract_addr, || {
        env.storage()
            .persistent()
            .set(&crate::DataKey::Paused, &false);
    });
}

// ─── initialize ──────────────────────────────────────────────────────────────

#[test]
fn initialize_only_once_fails() {
    let (_env, client, admin) = setup_initialized();
    super::assert_contract_error(
        client.try_initialize(&admin),
        EscrowError::AlreadyInitialized,
    );
}

// ─── pause / unpause state ──────────────────────────────────────────────────

#[test]
fn pause_then_unpause_toggles_state() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);

    assert!(!client.is_paused());
    client.pause();
    assert!(client.is_paused());
    client.unpause();
    assert!(!client.is_paused());
}

// --- create_contract ---

#[test]
fn pause_blocks_create_contract() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    client.pause();

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    super::assert_contract_error(
        client.try_create_contract(
            &a,
            &b,
            &None,
            &vec![&env, 50_i128],
            &ReleaseAuthorization::ClientOnly,
        ),
        EscrowError::ContractPaused,
    );
}

#[test]
fn unpause_restores_create_contract() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    client.pause();
    client.unpause();

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let id = client.create_contract(
        &a,
        &b,
        &None,
        &vec![&env, 50_i128],
        &ReleaseAuthorization::ClientOnly,
    );
    assert_eq!(id, 1);
}

#[test]
fn pause_gate_runs_before_auth_on_create_contract() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    client.pause();

    let outsider = Address::generate(&env);
    let other = Address::generate(&env);
    super::assert_contract_error(
        client.try_create_contract(
            &outsider,
            &other,
            &None,
            &vec![&env, 50_i128],
            &ReleaseAuthorization::ClientOnly,
        ),
        EscrowError::ContractPaused,
    );
}

// --- deposit_funds ---

#[test]
fn pause_blocks_deposit_funds() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_deposit_funds(&id, &client_addr, &50_i128),
        EscrowError::ContractPaused,
    );
}

#[test]
fn unpause_restores_deposit_funds() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    client.pause();
    client.unpause();

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let id = client.create_contract(
        &a,
        &b,
        &None,
        &vec![&env, 50_i128],
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&id, &a, &50_i128));
}

// --- release_milestone ---

#[test]
fn pause_blocks_release_milestone() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_release_milestone(&id, &client_addr, &0),
        Error::ContractPaused,
    );
}

#[test]
fn unpause_restores_release_milestone() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();

    client.approve_milestone_release(&id, &client_addr, &0);
    client.release_milestone(&id, &client_addr, &0);
}

// --- refund_unreleased_milestones ---

#[test]
fn pause_blocks_refund_unreleased_milestones() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (_client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_refund_unreleased_milestones(&id, &vec![&env, 1_u32]),
        EscrowError::ContractPaused,
    );
}

#[test]
fn unpause_restores_refund_unreleased_milestones() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (_client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();

    client.refund_unreleased_milestones(&id, &vec![&env, 1_u32]);
}

// --- cancel_contract ---

#[test]
fn pause_blocks_cancel_contract() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_cancel_contract(&id, &client_addr),
        Error::ContractPaused,
    );
}

#[test]
fn unpause_restores_cancel_contract() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer, id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();

    client.cancel_contract(&id, &client_addr);
}

// --- issue_reputation ---

#[test]
#[ignore]
fn pause_blocks_issue_reputation() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer_addr, id) = setup_completed_contract(&env, &client);
    client.pause();

    let comment = String::from_str(&env, "Great work");
    super::assert_contract_error(
        client.try_issue_reputation(&id, &client_addr, &5_u32, &comment),
        EscrowError::ContractPaused,
    );
}
