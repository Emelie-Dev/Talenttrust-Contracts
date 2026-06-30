//! Tests for the pause + emergency gate on mutating escrow entrypoints.
//!
//! Issue #452: `create_contract`, `deposit_funds`, `release_milestone`,
//! `refund_unreleased_milestones`, `issue_reputation`, and `cancel_contract`
//! must all honor the `Paused` and `Emergency` flags so the documented
//! behavior holds. Read-only queries must remain available while paused.
//!
//! Each mutating entrypoint is exercised against three states:
//! 1. **Paused only**: `pause()` blocks the call with `ContractPaused`.
//! 2. **Emergency only** (Paused=false, Emergency=true): the call is blocked
//!    with `EmergencyActive`.
//! 3. **Recovered**: `unpause()` (or `resolve_emergency()`) restores the
//!    happy path.
//!
//! Run locally with `cargo test -p escrow --lib pause_controls`.

use crate::{Error, Escrow, EscrowClient, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn setup_initialized() -> (Env, EscrowClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    (env, admin)
}

fn make_client<'a>(env: &'a Env, admin: &Address) -> EscrowClient<'a> {
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &contract_id);
    assert!(client.initialize(admin));
    client
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
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    assert!(!client.is_paused());
    assert!(client.pause());
    assert!(client.is_paused());
    assert!(client.unpause());
    assert!(!client.is_paused());
}

#[test]
fn pause_requires_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);
    super::assert_contract_error(client.try_pause(), Error::NotInitialized);
}

// ─── create_contract blocked ─────────────────────────────────────────────────

#[test]
fn pause_blocks_create_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
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
        Error::ContractPaused,
    );
}

#[test]
fn emergency_blocks_create_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    set_emergency_only(&env, &client);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    super::assert_contract_error(
        client.try_deposit_funds(&id, &caller, &50_i128),
        EscrowError::ContractPaused,
    );
    super::assert_contract_error(
        client.try_deposit_funds(&id, &client_addr, &50_i128),
        EscrowError::ContractPaused,
    );
}

// ─── approve_milestone_release blocked ───────────────────────────────────────

#[test]
fn pause_blocks_approve_milestone_release() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_approve_milestone_release(&contract_id, &client_addr, &0),
        Error::ContractPaused,
    );
    assert!(client.get_milestone_approvals(&contract_id, &0).is_none());
}

#[test]
fn emergency_blocks_approve_milestone_release() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);

    super::assert_contract_error(
        client.try_approve_milestone_release(&contract_id, &client_addr, &0),
        Error::EmergencyActive,
    );
    assert!(client.get_milestone_approvals(&contract_id, &0).is_none());
}

#[test]
fn unpause_restores_approve_milestone_release() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.get_milestone_approvals(&contract_id, &0).is_some());
}

#[test]
fn resolve_emergency_restores_approve_milestone_release() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.get_milestone_approvals(&contract_id, &0).is_some());
}

// ─── release_milestone blocked ───────────────────────────────────────────────

#[test]
fn pause_blocks_release_milestone() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _, id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_release_milestone(&id, &caller, &0),
        EscrowError::ContractPaused,
    );
    super::assert_contract_error(
        client.try_release_milestone(&id, &client_addr, &0),
        EscrowError::ContractPaused,
    );
}

// ─── issue_reputation blocked ────────────────────────────────────────────────

#[test]
#[ignore]
fn pause_blocks_issue_reputation() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _freelancer_addr, id) = setup_completed_contract(&env, &client);
    client.pause();

    let comment = soroban_sdk::String::from_str(&env, "Good job");
    super::assert_contract_error(
        client.try_issue_reputation(&id, &client_addr, &5_u32, &comment),
        EscrowError::ContractPaused,
    );
}

// ─── cancel_contract blocked ─────────────────────────────────────────────────

#[test]
fn pause_blocks_cancel_contract() {
    let (env, contract_id, _admin) = setup_initialized();
    let client = EscrowClient::new(&env, &contract_id);
    let (client_addr, _, id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_cancel_contract(&id, &client_addr),
        EscrowError::ContractPaused,
    );
}

// ─── unpaused allows operations ──────────────────────────────────────────────

#[test]
fn unpause_restores_create_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
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
fn resolve_emergency_restores_create_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());

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

// ─── deposit_funds blocked ───────────────────────────────────────────────────

#[test]
fn pause_blocks_deposit_funds() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_deposit_funds(&contract_id, &client_addr, &50_i128),
        Error::ContractPaused,
    );
}

#[test]
fn emergency_blocks_deposit_funds() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);

    super::assert_contract_error(
        client.try_deposit_funds(&contract_id, &client_addr, &50_i128),
        Error::EmergencyActive,
    );
}

#[test]
fn unpause_restores_deposit_funds() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_created_contract(&env, &client);
    client.pause();
    client.unpause();
    assert!(client.deposit_funds(&contract_id, &client_addr, &50_i128));
}

#[test]
fn resolve_emergency_restores_deposit_funds() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_created_contract(&env, &client);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());
    assert!(client.deposit_funds(&contract_id, &client_addr, &50_i128));
}

// ─── release_milestone blocked ───────────────────────────────────────────────

#[test]
fn _duplicate_pause_blocks_release_milestone() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_release_milestone(&contract_id, &client_addr, &0),
        Error::ContractPaused,
    );
}

#[test]
fn emergency_blocks_release_milestone() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);

    super::assert_contract_error(
        client.try_release_milestone(&contract_id, &client_addr, &0),
        Error::EmergencyActive,
    );
}

#[test]
fn unpause_restores_release_milestone() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
}

#[test]
fn resolve_emergency_restores_release_milestone() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
}

// ─── refund_unreleased_milestones blocked ───────────────────────────────────

#[test]
fn pause_blocks_refund_unreleased_milestones() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    match client.try_refund_unreleased_milestones(&contract_id, &vec![&env, 1_u32]) {
        Err(Ok(e)) => assert_eq!(e, soroban_sdk::Error::from(Error::ContractPaused)),
        other => panic!("expected ContractPaused error, got {:?}", other),
    }
}

#[test]
fn emergency_blocks_refund_unreleased_milestones() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);

    match client.try_refund_unreleased_milestones(&contract_id, &vec![&env, 1_u32]) {
        Err(Ok(e)) => assert_eq!(e, soroban_sdk::Error::from(Error::EmergencyActive)),
        other => panic!("expected EmergencyActive error, got {:?}", other),
    }
}

#[test]
fn unpause_restores_refund_unreleased_milestones() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();
    let total = client.refund_unreleased_milestones(&contract_id, &vec![&env, 1_u32]);
    assert_eq!(total, 200_i128);
}

#[test]
fn resolve_emergency_restores_refund_unreleased_milestones() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());
    let total = client.refund_unreleased_milestones(&contract_id, &vec![&env, 1_u32]);
    assert_eq!(total, 200_i128);
}

// ─── issue_reputation blocked ────────────────────────────────────────────────

#[test]
fn _duplicate_pause_blocks_issue_reputation() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _freelancer_addr, contract_id) = setup_completed_contract(&env, &client);
    client.pause();

    let comment = soroban_sdk::String::from_str(&env, "Good job");
    super::assert_contract_error(
        client.try_issue_reputation(
            &contract_id,
            &client_addr,
            &5_u32,
            &soroban_sdk::String::from_str(&env, "Great"),
        ),
        Error::ContractPaused,
    );
}

#[test]
fn emergency_blocks_issue_reputation() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _freelancer_addr, contract_id) = setup_completed_contract(&env, &client);
    set_emergency_only(&env, &client);

    let comment = soroban_sdk::String::from_str(&env, "Good job");
    super::assert_contract_error(
        client.try_issue_reputation(
            &contract_id,
            &client_addr,
            &5_u32,
            &soroban_sdk::String::from_str(&env, "Great"),
        ),
        Error::EmergencyActive,
    );
}

#[test]
fn unpause_restores_issue_reputation() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _freelancer_addr, contract_id) = setup_completed_contract(&env, &client);
    client.pause();
    client.unpause();

    let comment = soroban_sdk::String::from_str(&env, "Good job");
    assert!(client.issue_reputation(&contract_id, &client_addr, &5_u32, &comment));
}

#[test]
fn resolve_emergency_restores_issue_reputation() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _freelancer_addr, contract_id) = setup_completed_contract(&env, &client);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());

    let comment = soroban_sdk::String::from_str(&env, "Good job");
    assert!(client.issue_reputation(&contract_id, &client_addr, &5_u32, &comment));
}

// ─── cancel_contract blocked ─────────────────────────────────────────────────

#[test]
fn _duplicate_pause_blocks_cancel_contract() {
    let (env, client, _admin) = setup_initialized();
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    super::assert_contract_error(
        client.try_cancel_contract(&contract_id, &client_addr),
        Error::ContractPaused,
    );
}

#[test]
fn emergency_blocks_cancel_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);

    super::assert_contract_error(
        client.try_cancel_contract(&contract_id, &client_addr),
        Error::EmergencyActive,
    );
}

#[test]
fn unpause_restores_cancel_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();
    client.unpause();
    assert!(client.cancel_contract(&contract_id, &client_addr));
}

#[test]
fn resolve_emergency_restores_cancel_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);
    assert!(client.resolve_emergency());
    assert!(client.cancel_contract(&contract_id, &client_addr));
}

// ─── Read-only queries remain available while paused ────────────────────────

#[test]
fn read_only_queries_unaffected_by_pause() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    let _ = client.is_paused();
    let _ = client.is_emergency();
    let _ = client.get_contract(&contract_id);
    let _ = client.get_milestones(&contract_id);
    let _ = client.get_refundable_balance(&contract_id);
    let _ = client.get_admin();
}

#[test]
fn read_only_queries_unaffected_by_emergency() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    set_emergency_only(&env, &client);

    assert!(client.is_emergency());
    assert!(!client.is_paused());
    let _ = client.get_contract(&contract_id);
    let _ = client.get_milestones(&contract_id);
    let _ = client.get_refundable_balance(&contract_id);
    let _ = client.get_admin();
}

// ─── Cross-check: pause runs BEFORE auth (cycle-safe) ──────────────────────

#[test]
fn pause_gate_runs_before_auth_on_create_contract() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    client.pause();

    let outsider = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let milestones = vec![&env, 50_i128];

    super::assert_contract_error(
        client.try_create_contract(
            &outsider,
            &client_addr,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        ),
        Error::ContractPaused,
    );
}

#[test]
fn pause_gate_runs_before_auth_on_deposit_funds() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    let outsider = Address::generate(&env);
    super::assert_contract_error(
        client.try_deposit_funds(&contract_id, &outsider, &50_i128),
        Error::ContractPaused,
    );
}

#[test]
fn pause_gate_runs_before_auth_on_approve_milestone_release() {
    let (env, admin) = setup_initialized();
    let client = make_client(&env, &admin);
    let (_client_addr, _freelancer_addr, contract_id) = setup_funded_contract(&env, &client);
    client.pause();

    let outsider = Address::generate(&env);
    super::assert_contract_error(
        client.try_approve_milestone_release(&contract_id, &outsider, &0),
        Error::ContractPaused,
    );
    assert!(client.get_milestone_approvals(&contract_id, &0).is_none());
}
