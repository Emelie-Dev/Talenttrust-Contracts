use super::{default_milestones, generated_participants, register_client};

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env};

use crate::{ContractStatus, ReleaseAuthorization};

fn make_client_freelancer(env: &Env) -> (Address, Address) {
    generated_participants(env)
}

// ── Participant index tests ───────────────────────────────────────────────────

#[test]
fn participant_index_empty_returns_empty_page() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let participant = Address::generate(&env);

    let page_client = client.list_contracts_by_participant(&participant, &0u32, &0u32, &10u32);
    assert_eq!(page_client.len(), 0);

    let page_freelancer =
        client.list_contracts_by_participant(&participant, &1u32, &0u32, &10u32);
    assert_eq!(page_freelancer.len(), 0);
}

#[test]
fn participant_index_client_and_freelancer_lists_are_correct_and_paginated() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let (client1, freelancer1) = make_client_freelancer(&env);
    let (client2, freelancer2) = make_client_freelancer(&env);

    // Create two contracts.
    let milestones = default_milestones(&env);

    let id1 = escrow.create_contract(
        &client1,
        &freelancer1,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    let id2 = escrow.create_contract(
        &client2,
        &freelancer2,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    // Client pagination for client1: should contain only id1.
    let page = escrow.list_contracts_by_participant(&client1, &0u32, &0u32, &10u32);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0), id1);

    // Freelancer pagination for freelancer2: should contain only id2.
    let page = escrow.list_contracts_by_participant(&freelancer2, &1u32, &0u32, &10u32);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0), id2);

    // start out of range -> empty
    let page = escrow.list_contracts_by_participant(&client1, &0u32, &5u32, &10u32);
    assert_eq!(page.len(), 0);

    // limit cap behavior: request more than available; should return remaining only.
    let page = escrow.list_contracts_by_participant(&client1, &0u32, &0u32, &1000u32);
    assert_eq!(page.len(), 1);
}

#[test]
fn participant_index_multiple_contracts_same_client() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let client = Address::generate(&env);
    let milestones = default_milestones(&env);

    let mut ids = soroban_sdk::Vec::new(&env);
    for _ in 0..3u32 {
        let freelancer = Address::generate(&env);
        let id = escrow.create_contract(
            &client,
            &freelancer,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        );
        ids.push_back(id);
    }

    // All 3 contracts show up in the client list.
    let page = escrow.list_contracts_by_participant(&client, &0u32, &0u32, &50u32);
    assert_eq!(page.len(), 3);

    // Pagination: first 2.
    let page = escrow.list_contracts_by_participant(&client, &0u32, &0u32, &2u32);
    assert_eq!(page.len(), 2);

    // Pagination: last 1 via start=2.
    let page = escrow.list_contracts_by_participant(&client, &0u32, &2u32, &10u32);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0), ids.get(2));
}

#[test]
fn participant_index_limit_capped_at_max() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let client = Address::generate(&env);
    let milestones = default_milestones(&env);
    for _ in 0..5u32 {
        let freelancer = Address::generate(&env);
        escrow.create_contract(
            &client,
            &freelancer,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        );
    }

    // limit=1000 is capped at MAX_PAGE_LIMIT (50); 5 < 50 so all 5 are returned.
    let page = escrow.list_contracts_by_participant(&client, &0u32, &0u32, &1000u32);
    assert_eq!(page.len(), 5);
}

// ── Status index tests ────────────────────────────────────────────────────────

#[test]
fn status_index_new_contract_is_in_created() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let (client, freelancer) = make_client_freelancer(&env);
    let milestones = default_milestones(&env);
    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    let created = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &10u32);
    assert_eq!(created.len(), 1);
    assert_eq!(created.get(0), id);

    // Not in any other status yet.
    assert_eq!(
        escrow
            .list_contracts_by_status(&ContractStatus::Funded, &0u32, &10u32)
            .len(),
        0
    );
}

#[test]
fn status_index_empty_status_returns_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let page = escrow.list_contracts_by_status(&ContractStatus::Disputed, &0u32, &10u32);
    assert_eq!(page.len(), 0);

    let page = escrow.list_contracts_by_status(&ContractStatus::Completed, &0u32, &10u32);
    assert_eq!(page.len(), 0);
}

#[test]
fn status_index_deposit_transitions_created_to_funded() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &total);

    // Should now be Funded, not Created.
    let funded = escrow.list_contracts_by_status(&ContractStatus::Funded, &0u32, &10u32);
    assert_eq!(funded.len(), 1);
    assert_eq!(funded.get(0), id);

    let created = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &10u32);
    assert_eq!(created.len(), 0);
}

#[test]
fn status_index_partial_deposit_moves_to_partially_funded() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);
    let partial = total / 2;

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &partial);

    let partially_funded =
        escrow.list_contracts_by_status(&ContractStatus::PartiallyFunded, &0u32, &10u32);
    assert_eq!(partially_funded.len(), 1);
    assert_eq!(partially_funded.get(0), id);

    let created = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &10u32);
    assert_eq!(created.len(), 0);
}

#[test]
fn status_index_cancel_moves_to_cancelled() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let (client, freelancer) = make_client_freelancer(&env);
    let milestones = default_milestones(&env);
    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    escrow.cancel_contract(&id, &client);

    let cancelled = escrow.list_contracts_by_status(&ContractStatus::Cancelled, &0u32, &10u32);
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled.get(0), id);

    let created = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &10u32);
    assert_eq!(created.len(), 0);
}

#[test]
fn status_index_raise_dispute_moves_to_disputed() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let arbiter = Address::generate(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &Some(arbiter.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &total);
    escrow.raise_dispute(&id, &client);

    let disputed = escrow.list_contracts_by_status(&ContractStatus::Disputed, &0u32, &10u32);
    assert_eq!(disputed.len(), 1);
    assert_eq!(disputed.get(0), id);

    let funded = escrow.list_contracts_by_status(&ContractStatus::Funded, &0u32, &10u32);
    assert_eq!(funded.len(), 0);
}

#[test]
fn status_index_full_release_moves_to_completed() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &total);

    for idx in 0..milestones.len() {
        escrow.approve_milestone_release(&id, &client, &idx);
        escrow.release_milestone(&id, &client, &idx);
    }

    let completed = escrow.list_contracts_by_status(&ContractStatus::Completed, &0u32, &10u32);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed.get(0), id);

    let funded = escrow.list_contracts_by_status(&ContractStatus::Funded, &0u32, &10u32);
    assert_eq!(funded.len(), 0);
}

#[test]
fn status_index_refund_all_moves_to_refunded() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &total);

    // Refund all milestones.
    let indices = soroban_sdk::vec![&env, 0u32, 1u32, 2u32];
    escrow.refund_unreleased_milestones(&id, &indices);

    let refunded = escrow.list_contracts_by_status(&ContractStatus::Refunded, &0u32, &10u32);
    assert_eq!(refunded.len(), 1);
    assert_eq!(refunded.get(0), id);

    let funded = escrow.list_contracts_by_status(&ContractStatus::Funded, &0u32, &10u32);
    assert_eq!(funded.len(), 0);
}

#[test]
fn status_index_multiple_contracts_different_statuses() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    // Contract 1: stays Created.
    let (c1, f1) = make_client_freelancer(&env);
    let id1 = escrow.create_contract(
        &c1, &f1, &None, &milestones, &ReleaseAuthorization::ClientOnly,
    );

    // Contract 2: funded.
    let (c2, f2) = make_client_freelancer(&env);
    let id2 = escrow.create_contract(
        &c2, &f2, &None, &milestones, &ReleaseAuthorization::ClientOnly,
    );
    StellarAssetClient::new(&env, &token).mint(&c2, &total);
    escrow.deposit_funds(&id2, &c2, &total);

    // Contract 3: cancelled.
    let (c3, f3) = make_client_freelancer(&env);
    let id3 = escrow.create_contract(
        &c3, &f3, &None, &milestones, &ReleaseAuthorization::ClientOnly,
    );
    escrow.cancel_contract(&id3, &c3);

    // Verify each bucket.
    let created = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &10u32);
    assert_eq!(created.len(), 1);
    assert_eq!(created.get(0), id1);

    let funded = escrow.list_contracts_by_status(&ContractStatus::Funded, &0u32, &10u32);
    assert_eq!(funded.len(), 1);
    assert_eq!(funded.get(0), id2);

    let cancelled = escrow.list_contracts_by_status(&ContractStatus::Cancelled, &0u32, &10u32);
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled.get(0), id3);
}

#[test]
fn status_index_pagination_start_and_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let milestones = default_milestones(&env);

    // Create 5 contracts, all staying in Created.
    let mut ids = soroban_sdk::Vec::new(&env);
    for _ in 0..5u32 {
        let (c, f) = make_client_freelancer(&env);
        let id = escrow.create_contract(
            &c, &f, &None, &milestones, &ReleaseAuthorization::ClientOnly,
        );
        ids.push_back(id);
    }

    // First page of 2.
    let page = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &2u32);
    assert_eq!(page.len(), 2);
    assert_eq!(page.get(0), ids.get(0));
    assert_eq!(page.get(1), ids.get(1));

    // Second page of 2.
    let page = escrow.list_contracts_by_status(&ContractStatus::Created, &2u32, &2u32);
    assert_eq!(page.len(), 2);
    assert_eq!(page.get(0), ids.get(2));
    assert_eq!(page.get(1), ids.get(3));

    // Third page: only 1 remaining.
    let page = escrow.list_contracts_by_status(&ContractStatus::Created, &4u32, &2u32);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0), ids.get(4));

    // start out of range.
    let page = escrow.list_contracts_by_status(&ContractStatus::Created, &10u32, &2u32);
    assert_eq!(page.len(), 0);

    // limit 0 -> empty.
    let page = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &0u32);
    assert_eq!(page.len(), 0);
}

#[test]
fn status_index_limit_capped_at_max_page_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let milestones = default_milestones(&env);
    for _ in 0..5u32 {
        let (c, f) = make_client_freelancer(&env);
        escrow.create_contract(&c, &f, &None, &milestones, &ReleaseAuthorization::ClientOnly);
    }

    // limit=1000 is capped; 5 < MAX_PAGE_LIMIT so all 5 are returned.
    let page = escrow.list_contracts_by_status(&ContractStatus::Created, &0u32, &1000u32);
    assert_eq!(page.len(), 5);
}

#[test]
fn status_index_dispute_resolution_full_refund_moves_to_refunded() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let arbiter = Address::generate(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &Some(arbiter.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &total);
    escrow.raise_dispute(&id, &client);

    escrow.resolve_dispute(&id, &arbiter, &crate::DisputeResolution::FullRefund);

    let disputed = escrow.list_contracts_by_status(&ContractStatus::Disputed, &0u32, &10u32);
    assert_eq!(disputed.len(), 0);

    let refunded = escrow.list_contracts_by_status(&ContractStatus::Refunded, &0u32, &10u32);
    assert_eq!(refunded.len(), 1);
    assert_eq!(refunded.get(0), id);
}

#[test]
fn status_index_dispute_resolution_full_payout_moves_to_completed() {
    let env = Env::default();
    env.mock_all_auths();

    let escrow_addr = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &escrow_addr);
    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let token = env.register_stellar_asset_contract(admin.clone());
    escrow.bind_settlement_token(&admin, &token);

    let (client, freelancer) = make_client_freelancer(&env);
    let arbiter = Address::generate(&env);
    let milestones = default_milestones(&env);
    let total: i128 = milestones.iter().fold(0, |s, m| s + m);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &Some(arbiter.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    StellarAssetClient::new(&env, &token).mint(&client, &total);
    escrow.deposit_funds(&id, &client, &total);
    escrow.raise_dispute(&id, &client);

    escrow.resolve_dispute(&id, &arbiter, &crate::DisputeResolution::FullPayout);

    let disputed = escrow.list_contracts_by_status(&ContractStatus::Disputed, &0u32, &10u32);
    assert_eq!(disputed.len(), 0);

    let completed = escrow.list_contracts_by_status(&ContractStatus::Completed, &0u32, &10u32);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed.get(0), id);
}
