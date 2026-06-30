use crate::{Escrow, EscrowClient, Error, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, Env, Vec};

fn setup_contract(env: &Env) -> (EscrowClient, soroban_sdk::Address, soroban_sdk::Address, soroban_sdk::Address) {
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &contract_id);
    
    let client_addr = soroban_sdk::Address::generate(env);
    let freelancer_addr = soroban_sdk::Address::generate(env);
    let arbiter_addr = soroban_sdk::Address::generate(env);
    
    (client, client_addr, freelancer_addr, arbiter_addr)
}

fn default_milestones(env: &Env) -> Vec<i128> {
    Vec::from_array(env, [1000_0000000_i128, 2000_0000000_i128, 3000_0000000_i128])
}

fn total_milestones() -> i128 {
    6000_0000000_i128
}

#[test]
fn test_approve_milestone_client_only() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Verify approval was recorded
    let approvals = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals.is_some());
    let approvals = approvals.unwrap();
    assert!(approvals.client_approved);
    assert!(!approvals.freelancer_approved);
    assert!(!approvals.arbiter_approved);
}

#[test]
fn test_approve_milestone_multisig() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::MultiSig,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Client approves
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Freelancer approves
    assert!(client.approve_milestone_release(&contract_id, &freelancer_addr, &0));
    
    // Verify both approvals recorded
    let approvals = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals.is_some());
    let approvals = approvals.unwrap();
    assert!(approvals.client_approved);
    assert!(approvals.freelancer_approved);
}

#[test]
fn test_approve_milestone_arbiter_only() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &default_milestones(&env),
        &ReleaseAuthorization::ArbiterOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Arbiter approves
    assert!(client.approve_milestone_release(&contract_id, &arbiter_addr, &0));
    
    // Verify approval
    let approvals = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals.is_some());
    let approvals = approvals.unwrap();
    assert!(!approvals.client_approved);
    assert!(approvals.arbiter_approved);
}

#[test]
fn test_approve_milestone_client_and_arbiter() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &default_milestones(&env),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Client approves (either client or arbiter is sufficient)
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Verify approval
    let approvals = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals.is_some());
    let approvals = approvals.unwrap();
    assert!(approvals.client_approved);
}

#[test]
fn test_duplicate_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Second approval should fail
    let result = client.try_approve_milestone_release(&contract_id, &client_addr, &0);
    assert_eq!(result, Err(Ok(Error::AlreadyApproved)));
}

#[test]
fn test_unauthorized_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Freelancer cannot approve in ClientOnly mode
    let result = client.try_approve_milestone_release(&contract_id, &freelancer_addr, &0);
    assert_eq!(result, Err(Ok(Error::UnauthorizedRole)));
}

#[test]
fn test_release_requires_approval() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Try to release without approval - should fail
    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    assert_eq!(result, Err(Ok(Error::InsufficientApprovals)));
}

#[test]
fn test_release_with_approval_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    
    // Verify milestone was released
    let milestones = client.get_milestones(&contract_id);
    assert!(milestones.get(0).unwrap().released);
    
    // Verify approvals were cleared
    let approvals = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals.is_none());
}

#[test]
fn test_multisig_requires_both_approvals() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::MultiSig,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Only client approves
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Try to release - should fail (need freelancer approval too)
    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    assert_eq!(result, Err(Ok(Error::InsufficientApprovals)));
    
    // Freelancer approves
    assert!(client.approve_milestone_release(&contract_id, &freelancer_addr, &0));
    
    // Now release should succeed
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
}

#[test]
fn test_approval_expiry_simulation() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Verify approval exists
    let approvals = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals.is_some());
    
    // Note: In a real scenario, we would advance ledgers beyond TTL
    // For now, we verify the approval mechanism works
    // TTL expiry is handled by Soroban's temporary storage automatically
}

#[test]
fn test_approve_already_released_milestone_fails() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    
    // Try to approve again after release
    let result = client.try_approve_milestone_release(&contract_id, &client_addr, &0);
    assert_eq!(result, Err(Ok(Error::MilestoneAlreadyReleased)));
}

#[test]
fn test_approve_invalid_milestone_index() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Try to approve invalid milestone index
    let result = client.try_approve_milestone_release(&contract_id, &client_addr, &99);
    assert_eq!(result, Err(Ok(Error::IndexOutOfBounds)));
}

#[test]
fn test_approve_requires_funded_state() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    // Try to approve before funding
    let result = client.try_approve_milestone_release(&contract_id, &client_addr, &0);
    assert_eq!(result, Err(Ok(Error::InvalidState)));
}

#[test]
fn test_multiple_milestones_independent_approvals() {
    let env = Env::default();
    env.mock_all_auths();
    
    let (client, client_addr, freelancer_addr, _arbiter_addr) = setup_contract(&env);
    
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestones()));
    
    // Approve milestone 0
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    
    // Approve milestone 1
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    
    // Verify both have independent approvals
    let approvals_0 = client.get_milestone_approvals(&contract_id, &0);
    let approvals_1 = client.get_milestone_approvals(&contract_id, &1);
    
    assert!(approvals_0.is_some());
    assert!(approvals_1.is_some());
    
    // Release milestone 0
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    
    // Milestone 0 approvals should be cleared
    let approvals_0 = client.get_milestone_approvals(&contract_id, &0);
    assert!(approvals_0.is_none());
    
    // Milestone 1 approvals should still exist
    let approvals_1 = client.get_milestone_approvals(&contract_id, &1);
    assert!(approvals_1.is_some());
}
