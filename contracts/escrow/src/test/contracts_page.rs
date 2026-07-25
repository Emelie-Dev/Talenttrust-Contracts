#![cfg(test)]
use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};
use crate::types::{ContractEntry, ContractStatus};

#[test]
fn empty_page() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    
    let admin = Address::generate(&env);
    let escrow_address = Address::generate(&env);
    let client = EscrowClient::new(&env, &escrow_address);
    client.initialize(&admin, &100, &100_000_000_000, &Address::generate(&env));
    
    // Total = 0
    let page = client.get_contracts_page(&0, &10);
    assert_eq!(page.len(), 0);
}

#[test]
fn single_page() {
    let mut fix = EscrowFixture::builder().funded(true).build();
    let page = fix.escrow().get_contracts_page(&0, &10);
    assert_eq!(page.len(), 1);
    
    let entry = page.get(0).unwrap();
    assert_eq!(entry.id, fix.escrow_id);
    assert_eq!(entry.client, fix.client);
    assert_eq!(entry.freelancer, fix.freelancer);
    assert_eq!(entry.status, ContractStatus::Funded);
}

#[test]
fn pagination_continuation() {
    let mut fix1 = EscrowFixture::builder().funded(true).build();
    
    // Create a second contract
    let client = Address::generate(&fix1.env);
    let freelancer = Address::generate(&fix1.env);
    let milestones = vec![&fix1.env, 1000];
    
    fix1.env.mock_all_auths();
    fix1.escrow().create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    
    // Now there are 2 contracts
    // Get first page of size 1
    let p1 = fix1.escrow().get_contracts_page(&0, &1);
    assert_eq!(p1.len(), 1);
    assert_eq!(p1.get(0).unwrap().id, 1);
    
    // Get second page
    let p2 = fix1.escrow().get_contracts_page(&1, &1);
    assert_eq!(p2.len(), 1);
    assert_eq!(p2.get(0).unwrap().id, 2);
}

#[test]
fn ceiling_clamp() {
    let mut fix = EscrowFixture::builder().funded(true).build();
    let client2 = Address::generate(&fix.env);
    let freelancer2 = Address::generate(&fix.env);
    let milestones = vec![&fix.env, 1000];
    
    // Create lots of contracts
    for _ in 0..60 {
        fix.escrow().create_contract(
            &client2,
            &freelancer2,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        );
    }
    
    // PAGE_CEILING is 50. Requesting 100 should clamp to 50.
    let page = fix.escrow().get_contracts_page(&0, &100);
    assert_eq!(page.len(), 50);
}
