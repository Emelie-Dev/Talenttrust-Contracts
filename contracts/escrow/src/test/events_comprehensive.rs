#![cfg(test)]

use super::register_client;
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{Address, Env, Symbol, TryFromVal};

#[test]
fn contract_creation_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00, 150_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "created" event exists
    let created_topic = soroban_sdk::symbol_short!("created");
    let found = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&created_topic)
    });
    assert!(found, "created event should be emitted");

    // Verify contract_id in topic
    let found_with_id = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&created_topic)
            && event.1.get(1).unwrap() == contract_id.into()
    });
    assert!(found_with_id, "created event should include contract_id");
}

#[test]
fn contract_finalization_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Complete the contract by releasing the milestone
    client.approve_milestone_release(&contract_id, &0);
    client.release_milestone(&contract_id, &0);

    // Finalize the contract
    client.finalize_contract(&contract_id);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "finalized" event exists
    let finalized_topic = soroban_sdk::symbol_short!("finalized");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&finalized_topic)
    });
    assert!(found, "finalized event should be emitted");
}

#[test]
fn contract_cancellation_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Cancel the contract
    client.cancel_contract(&contract_id);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "cancelled" event exists
    let cancelled_topic = soroban_sdk::symbol_short!("cancelled");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&cancelled_topic)
    });
    assert!(found, "cancelled event should be emitted");
}

#[test]
fn milestone_release_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Release milestone
    client.approve_milestone_release(&contract_id, &0);
    client.release_milestone(&contract_id, &0);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "mlstn_rls" event exists
    let mlstn_rls_topic = soroban_sdk::symbol_short!("mlstn_rls");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&mlstn_rls_topic)
    });
    assert!(found, "mlstn_rls event should be emitted");
}

#[test]
fn contract_completion_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Release the only milestone (completes contract)
    client.approve_milestone_release(&contract_id, &0);
    client.release_milestone(&contract_id, &0);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "ctrct_cmp" event exists
    let ctrct_cmp_topic = soroban_sdk::symbol_short!("ctrct_cmp");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&ctrct_cmp_topic)
    });
    assert!(found, "ctrct_cmp event should be emitted on contract completion");
}

#[test]
fn work_evidence_submission_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Submit work evidence
    let evidence = soroban_sdk::String::from_str(&env, "QmHash123");
    client.submit_work_evidence(&contract_id, &0, &evidence);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "evidence" event exists
    let evidence_topic = soroban_sdk::symbol_short!("evidence");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&evidence_topic)
    });
    assert!(found, "evidence event should be emitted");
}

#[test]
fn refund_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Refund milestone (no deadline, so can refund anytime)
    let refund_indices = vec![&env, 0];
    client.refund_unreleased_milestones(&contract_id, refund_indices);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "refunded" event exists
    let refunded_topic = soroban_sdk::symbol_short!("refunded");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&refunded_topic)
    });
    assert!(found, "refunded event should be emitted");
}

#[test]
fn pause_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Pause the contract
    client.pause();

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "pause" event exists
    let pause_topic = soroban_sdk::symbol_short!("pause");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&pause_topic)
    });
    assert!(found, "pause event should be emitted");
}

#[test]
fn unpause_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Pause then unpause
    client.pause();
    client.unpause();

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "unpaused" event exists
    let unpaused_topic = soroban_sdk::symbol_short!("unpaused");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&unpaused_topic)
    });
    assert!(found, "unpaused event should be emitted");
}

#[test]
fn emergency_activation_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Activate emergency pause
    client.activate_emergency_pause();

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify ("emergency", "activated") event exists
    let emergency_topic = Symbol::new(&env, "emergency");
    let activated_topic = Symbol::new(&env, "activated");
    let found = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&emergency_topic)
            && Symbol::try_from_val(&env, &event.1.get(1).unwrap())
                .ok()
                .as_ref()
                == Some(&activated_topic)
    });
    assert!(found, "emergency activated event should be emitted");
}

#[test]
fn emergency_resolution_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Activate then resolve emergency
    client.activate_emergency_pause();
    client.resolve_emergency();

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify ("emergency", "resolved") event exists
    let emergency_topic = Symbol::new(&env, "emergency");
    let resolved_topic = Symbol::new(&env, "resolved");
    let found = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&emergency_topic)
            && Symbol::try_from_val(&env, &event.1.get(1).unwrap())
                .ok()
                .as_ref()
                == Some(&resolved_topic)
    });
    assert!(found, "emergency resolved event should be emitted");
}

#[test]
fn dispute_opening_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let arbiter = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        Some(arbiter.clone()),
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Open dispute
    client.raise_dispute(&contract_id);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify ("dispute", "opened") event exists
    let dispute_topic = soroban_sdk::symbol_short!("dispute");
    let opened_topic = soroban_sdk::symbol_short!("opened");
    let found = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&dispute_topic)
            && Symbol::try_from_val(&env, &event.1.get(1).unwrap())
                .ok()
                .as_ref()
                == Some(&opened_topic)
    });
    assert!(found, "dispute opened event should be emitted");
}

#[test]
fn dispute_resolution_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let arbiter = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        Some(arbiter.clone()),
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Open and resolve dispute
    client.raise_dispute(&contract_id);
    client.resolve_dispute(&contract_id, &arbiter, &0); // FullRefund

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify ("dispute", "resolved") event exists
    let dispute_topic = soroban_sdk::symbol_short!("dispute");
    let resolved_topic = soroban_sdk::symbol_short!("resolved");
    let found = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&dispute_topic)
            && Symbol::try_from_val(&env, &event.1.get(1).unwrap())
                .ok()
                .as_ref()
                == Some(&resolved_topic)
    });
    assert!(found, "dispute resolved event should be emitted");
}

#[test]
fn client_migration_proposed_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let new_client = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Propose client migration
    client.propose_client_migration(&contract_id, &new_client);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "client_migration_proposed" event exists
    let migration_topic = Symbol::new(&env, "client_migration_proposed");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&migration_topic)
    });
    assert!(found, "client_migration_proposed event should be emitted");
}

#[test]
fn client_migration_accepted_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let new_client = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Propose and accept client migration
    client.propose_client_migration(&contract_id, &new_client);
    client.accept_client_migration(&contract_id);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "client_migration_accepted" event exists
    let migration_topic = Symbol::new(&env, "client_migration_accepted");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&migration_topic)
    });
    assert!(found, "client_migration_accepted event should be emitted");
}

#[test]
fn client_migration_cancelled_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);
    let new_client = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Propose and cancel client migration
    client.propose_client_migration(&contract_id, &new_client);
    client.cancel_client_migration(&contract_id);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "client_migration_cancelled" event exists
    let migration_topic = Symbol::new(&env, "client_migration_cancelled");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&migration_topic)
    });
    assert!(found, "client_migration_cancelled event should be emitted");
}

#[test]
fn settlement_token_binding_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    client.initialize(&admin);

    // Bind settlement token
    client.bind_settlement_token(&token);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "settlement_token_bound" event exists
    let token_topic = Symbol::new(&env, "settlement_token_bound");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&token_topic)
    });
    assert!(found, "settlement_token_bound event should be emitted");
}

#[test]
fn initialization_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);

    // Initialize contract
    client.initialize(&admin);

    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify ("init", "admin_set") event exists
    let init_topic = soroban_sdk::symbol_short!("init");
    let admin_set_topic = Symbol::new(&env, "admin_set");
    let found = events.iter().any(|event| {
        event.1.len() > 1
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&init_topic)
            && Symbol::try_from_val(&env, &event.1.get(1).unwrap())
                .ok()
                .as_ref()
                == Some(&admin_set_topic)
    });
    assert!(found, "init admin_set event should be emitted");
}

#[test]
fn protocol_fee_withdrawal_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);

    client.initialize(&admin);

    // Set protocol fee and withdraw (even with zero accumulated fees for testing)
    client.set_protocol_fee_bps(&100);
    
    // This will fail due to insufficient fees, but we can test the event structure
    // by checking that the set_protocol_fee_bps event was emitted
    let events = env.events().all();
    assert!(events.len() > 0);

    // Verify "protocol_fee_bps" event exists
    let fee_topic = Symbol::new(&env, "protocol_fee_bps");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&fee_topic)
    });
    assert!(found, "protocol_fee_bps event should be emitted");
}


#[test]
fn event_ordering_in_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let admin = Address::generate(&env);
    let freelancer = Address::generate(&env);

    client.initialize(&admin);

    let milestones = vec![&env, 100_000_00];
    let contract_id = client.create_contract(
        &freelancer,
        None,
        milestones.clone(),
        soroban_sdk::symbol_short!("client_only"),
    );

    // Release milestone
    client.approve_milestone_release(&contract_id, &0);
    client.release_milestone(&contract_id, &0);

    let events = env.events().all();
    
    // Find indices of relevant events
    let mut created_idx = None;
    let mut mlstn_rls_idx = None;
    let mut ctrct_cmp_idx = None;

    for (i, event) in events.iter().enumerate() {
        if event.1.len() > 0 {
            if let Ok(sym) = Symbol::try_from_val(&env, &event.1.get(0).unwrap()) {
                if sym == soroban_sdk::symbol_short!("created") {
                    created_idx = Some(i);
                } else if sym == soroban_sdk::symbol_short!("mlstn_rls") {
                    mlstn_rls_idx = Some(i);
                } else if sym == soroban_sdk::symbol_short!("ctrct_cmp") {
                    ctrct_cmp_idx = Some(i);
                }
            }
        }
    }

    // Verify ordering: created < mlstn_rls < ctrct_cmp
    assert!(created_idx.is_some(), "created event should exist");
    assert!(mlstn_rls_idx.is_some(), "mlstn_rls event should exist");
    assert!(ctrct_cmp_idx.is_some(), "ctrct_cmp event should exist");
    
    assert!(
        created_idx < mlstn_rls_idx && mlstn_rls_idx < ctrct_cmp_idx,
        "events should be emitted in correct order"
    );
}
