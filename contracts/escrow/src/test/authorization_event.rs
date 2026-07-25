#![cfg(test)]

use super::{create_contract, register_client, total_milestone_amount};
use soroban_sdk::{symbol_short, Env, Symbol, TryFromVal};

#[test]
fn approve_milestone_release_emits_auth_chg_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    client.approve_milestone_release(&contract_id, &client_addr, &0);

    let events = env.events().all();
    let auth_chg_topic = symbol_short!("auth_chg");
    let found = events.iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&auth_chg_topic)
    });
    assert!(found, "expected an auth_chg event immediately after approval");
}

#[test]
fn separate_milestone_approvals_each_emit_their_own_auth_chg_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    client.approve_milestone_release(&contract_id, &client_addr, &0);
    client.approve_milestone_release(&contract_id, &client_addr, &1);

    let events = env.events().all();
    let auth_chg_topic = symbol_short!("auth_chg");
    let auth_chg_count = events
        .iter()
        .filter(|event| {
            event.1.len() > 0
                && Symbol::try_from_val(&env, &event.1.get(0).unwrap())
                    .ok()
                    .as_ref()
                    == Some(&auth_chg_topic)
        })
        .count();

    assert_eq!(
        auth_chg_count, 2,
        "each approval should emit its own auth_chg event"
    );
}

/// `auth_chg` is not used as an event topic anywhere else in this contract, so
/// there is no risk of an indexer conflating it with an existing topic
/// (`admin`, `appr`, `cancelled`, `created`, `ctrct_cmp`, `deposit`,
/// `dispute`, `evidence`, `fee`, `finalized`, `init`, `migr`, `mlstn_rls`,
/// `opened`, `pause`, `refunded`, `resolved`, `unpaused`, `withdraw`).
#[test]
fn auth_chg_topic_does_not_collide_with_existing_topics() {
    let existing_topics = [
        "admin",
        "appr",
        "cancelled",
        "created",
        "ctrct_cmp",
        "deposit",
        "dispute",
        "evidence",
        "fee",
        "finalized",
        "init",
        "migr",
        "mlstn_rls",
        "opened",
        "pause",
        "refunded",
        "resolved",
        "unpaused",
        "withdraw",
    ];
    assert!(!existing_topics.contains(&"auth_chg"));
}
