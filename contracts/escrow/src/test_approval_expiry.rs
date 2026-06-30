#![cfg(test)]

use soroban_sdk::{testutils::Ledger as _, testutils::LedgerInfo, vec, Address, Env};

use crate::{Escrow, EscrowClient, EscrowError};

fn setup<'a>(env: &'a Env) -> (EscrowClient<'a>, Address, Address) {
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &contract_id);

    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);

    (client, client_addr, freelancer_addr)
}

#[test]
fn test_approval_expiry_success() {
    let env = Env::default();
    let (client, client_addr, freelancer_addr) = setup(&env);

    let milestones = vec![&env, 1000_i128];
    let expiry_window = 3600; // 1 hour
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &Some(expiry_window),
    );

    // Approve milestone
    client.approve_milestone(&contract_id, &0);

    // Fast forward 30 mins (within window)
    env.ledger().set(LedgerInfo {
        timestamp: 1800,
        protocol_version: 20,
        sequence_number: 100,
        network_id: [0u8; 32],
        base_reserve: 10,
    });

    // Release should succeed
    assert!(client.release_milestone(&contract_id, &0));
}

#[test]
fn test_approval_expiry_failure() {
    let env = Env::default();
    let (client, client_addr, freelancer_addr) = setup(&env);

    let milestones = vec![&env, 1000_i128];
    let expiry_window = 3600; // 1 hour
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &Some(expiry_window),
    );

    // Approve milestone at T=0
    client.approve_milestone(&contract_id, &0);

    // Fast forward 2 hours (past window)
    env.ledger().set(LedgerInfo {
        timestamp: 7200,
        protocol_version: 20,
        sequence_number: 100,
        network_id: [0u8; 32],
        base_reserve: 10,
    });

    // Release should fail with ApprovalExpired
    let result = client.try_release_milestone(&contract_id, &0);
    assert!(result.is_err());
}

#[test]
fn test_reapproval_resets_expiry() {
    let env = Env::default();
    let (client, client_addr, freelancer_addr) = setup(&env);

    let milestones = vec![&env, 1000_i128];
    let expiry_window = 3600; // 1 hour
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &Some(expiry_window),
    );

    // First approval at T=0
    client.approve_milestone(&contract_id, &0);

    // Fast forward 2 hours
    env.ledger().set(LedgerInfo {
        timestamp: 7200,
        protocol_version: 20,
        sequence_number: 100,
        network_id: [0u8; 32],
        base_reserve: 10,
    });

    // Release would fail now, so re-approve at T=7200
    client.approve_milestone(&contract_id, &0);

    // Fast forward another 30 mins
    env.ledger().set(LedgerInfo {
        timestamp: 7200 + 1800,
        protocol_version: 20,
        sequence_number: 200,
        network_id: [0u8; 32],
        base_reserve: 10,
    });

    // Release should succeed now
    assert!(client.release_milestone(&contract_id, &0));
}

#[test]
fn test_no_expiry_window_set() {
    let env = Env::default();
    let (client, client_addr, freelancer_addr) = setup(&env);

    let milestones = vec![&env, 1000_i128];
    // No expiry window set
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &None,
    );

    // Approve milestone at T=0
    client.approve_milestone(&contract_id, &0);

    // Fast forward 1 year
    env.ledger().set(LedgerInfo {
        timestamp: 365 * 24 * 3600,
        protocol_version: 20,
        sequence_number: 1000,
        network_id: [0u8; 32],
        base_reserve: 10,
    });

    // Release should still succeed
    assert!(client.release_milestone(&contract_id, &0));
}
