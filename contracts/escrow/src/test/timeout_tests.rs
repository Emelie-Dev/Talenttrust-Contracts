use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{vec, Address, Env};

use crate::{ContractStatus, Error, Escrow, EscrowClient, ReleaseAuthorization};

/// Helper: setup an initialized env with a funded contract where milestone 1 has a deadline.
fn setup_funded_with_deadline(
    env: &Env,
    deadline_ts: u64,
) -> (EscrowClient, Address, Address, u32) {
    env.mock_all_auths();

    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);

    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = vec![env, 200_0000000_i128, 400_0000000_i128, 600_0000000_i128];
    let deadlines = vec![env, 0_u64, deadline_ts, 0_u64];

    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
        &Some(deadlines),
    );

    client.deposit_funds(&id, &client_addr, &super::total_milestone_amount());

    (client, client_addr, freelancer_addr, id)
}

fn set_time(env: &Env, ts: u64) {
    env.ledger().set(LedgerInfo {
        timestamp: ts,
        ..Default::default()
    });
}

// ─────── deadline NOT passed ─────────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_before_deadline() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, 1000);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, Error::DeadlineNotPassed);
}

#[test]
fn claim_timeout_refund_fails_at_exact_deadline() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, Error::DeadlineNotPassed);
}

// ─────── deadline passed ─────────────────────────────────────────────────

#[test]
fn claim_timeout_refund_succeeds_after_deadline() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts + 1);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    let refunded = client.claim_timeout_refund(&id, &1_u32);
    assert_eq!(refunded, 400_0000000);

    let milestones = client.get_milestones(&id);
    let ms = milestones.get(1).unwrap();
    assert!(ms.refunded);
    assert!(!ms.released);

    let contract = client.get_contract(&id);
    assert_eq!(contract.refunded_amount, 400_0000000);
    assert_eq!(contract.status, ContractStatus::Funded);
}

#[test]
fn claim_timeout_refund_completes_contract_when_all_milestones_dealt() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, 500);
    let (client, client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    // Release milestones 0 and 2 (no deadline)
    client.approve_milestone_release(&id, &client_addr, &0);
    client.release_milestone(&id, &client_addr, &0);
    client.approve_milestone_release(&id, &client_addr, &2);
    client.release_milestone(&id, &client_addr, &2);

    // Now timeout-refund milestone 1 after deadline
    set_time(&env, deadline_ts + 1);
    let refunded = client.claim_timeout_refund(&id, &1_u32);
    assert_eq!(refunded, 400_0000000);

    let contract = client.get_contract(&id);
    assert_eq!(contract.status, ContractStatus::Completed);
}

// ─────── no deadline on milestone ────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_when_no_deadline_set() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 100_i128];
    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
        &None,
    );
    client.deposit_funds(&id, &client_addr, &100_i128);

    let result = client.try_claim_timeout_refund(&id, &0_u32);
    super::assert_contract_error(result, Error::DeadlineNotPassed);
}

// ─────── already released ───────────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_when_already_released() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, 500);
    let (client, client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    client.approve_milestone_release(&id, &client_addr, &1);
    client.release_milestone(&id, &client_addr, &1);

    set_time(&env, deadline_ts + 1);
    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, Error::AlreadyReleased);
}

// ─────── already refunded ───────────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_when_already_refunded() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts + 1);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    let _ = client.claim_timeout_refund(&id, &1_u32);

    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, Error::AlreadyRefunded);
}

// ─────── invalid index ──────────────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_out_of_bounds() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts + 1);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    let result = client.try_claim_timeout_refund(&id, &99_u32);
    super::assert_contract_error(result, Error::IndexOutOfBounds);
}

// ─────── unauthorized caller ────────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_when_freelancer_calls() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts + 1);
    let (client, _client_addr, freelancer_addr, id) = setup_funded_with_deadline(&env, deadline_ts);

    // Disable mock auth — Soroban will reject any unsigned call
    env.mock_all_auths();

    // Freelancer tries to claim timeout refund
    let env2 = env.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let client2 = EscrowClient::new(&env2, &client.address.clone());
        client2.claim_timeout_refund(&id, &1_u32);
    }));
    assert!(result.is_err(), "freelancer call should panic");
}

// ─────── pause gate ─────────────────────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_when_paused() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts + 1);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    client.pause();

    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, crate::EscrowError::ContractPaused);
}

// ─────── non-terminal state check ───────────────────────────────────────

#[test]
fn claim_timeout_refund_fails_in_completed_state() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, 500);
    let (client, client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    // Release all milestones
    for i in 0..3_u32 {
        client.approve_milestone_release(&id, &client_addr, &i);
        client.release_milestone(&id, &client_addr, &i);
    }

    set_time(&env, deadline_ts + 1);
    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, Error::InvalidState);
}

// ─────── multiple deadlines ─────────────────────────────────────────────

#[test]
fn claim_timeout_refund_works_only_on_individual_milestones() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 100_i128, 200_i128];
    let deadlines = vec![&env, 3000_u64, 4000_u64];

    set_time(&env, 1000);
    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
        &Some(deadlines),
    );
    client.deposit_funds(&id, &client_addr, &300_i128);

    // Only first milestone deadline passed
    set_time(&env, 3500);
    let r0 = client.claim_timeout_refund(&id, &0_u32);
    assert_eq!(r0, 100_i128);

    // Second deadline not yet passed
    let result = client.try_claim_timeout_refund(&id, &1_u32);
    super::assert_contract_error(result, Error::DeadlineNotPassed);

    // After second deadline passes
    set_time(&env, 5000);
    let r1 = client.claim_timeout_refund(&id, &1_u32);
    assert_eq!(r1, 200_i128);

    let contract = client.get_contract(&id);
    assert_eq!(contract.status, ContractStatus::Refunded);
}

// ─────── event emission ─────────────────────────────────────────────────

#[test]
fn claim_timeout_refund_emits_event() {
    let env = Env::default();
    let deadline_ts = 2000;

    set_time(&env, deadline_ts + 1);
    let (client, _client_addr, _freelancer, id) = setup_funded_with_deadline(&env, deadline_ts);

    client.claim_timeout_refund(&id, &1_u32);

    let events = env.events().all();
    let last = events.last().unwrap();
    let (_contract_ids, topics, _data) = last;

    assert_eq!(
        topics,
        (
            soroban_sdk::symbol_short!("timeout"),
            soroban_sdk::symbol_short!("refund")
        )
    );
}
