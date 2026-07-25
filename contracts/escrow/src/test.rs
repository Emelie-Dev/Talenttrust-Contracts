use soroban_sdk::{symbol_short, testutils::Address as _, vec, Address, Env};

use crate::{Escrow, EscrowClient, Milestone};

#[test]
fn test_hello() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let result = client.hello(&symbol_short!("World"));
    assert_eq!(result, symbol_short!("World"));
}

#[test]
fn test_create_contract() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 200_0000000_i128, 400_0000000_i128, 600_0000000_i128];

    let id = client.create_contract(&client_addr, &freelancer_addr, &milestones);
    assert_eq!(id, 1);
}

#[test]
fn test_deposit_funds() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 200_0000000_i128, 400_0000000_i128, 600_0000000_i128];
    let escrow_id = client.create_contract(&client_addr, &freelancer_addr, &milestones);

    let result = client.deposit_funds(&escrow_id, &1_000_0000000);
    assert!(result);
}

#[test]
fn test_release_milestone_event() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestone_amounts = vec![&env, 200_0000000_i128, 400_0000000_i128, 600_0000000_i128];
    let escrow_id = client.create_contract(&client_addr, &freelancer_addr, &milestone_amounts);
    client.deposit_funds(&escrow_id, &1_200_0000000);

    // Release first milestone
    let milestone_id = 0u32;
    client.release_milestone(&escrow_id, &milestone_id);

    // Verify event emission
    let events = env.events().all();
    assert_eq!(events.len(), 1);

    let event = events.get(0).unwrap();
    assert_eq!(event.0.len(), 3);
    assert_eq!(event.0.get(0).unwrap(), symbol_short!("ms_state"));
    assert_eq!(event.0.get(1).unwrap(), escrow_id);
    assert_eq!(event.0.get(2).unwrap(), milestone_id);

    let milestone: Milestone = event.1;
    assert_eq!(milestone.amount, 200_0000000);
    assert_eq!(milestone.released, true);
}
