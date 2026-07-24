#![cfg(test)]

use super::{has_event_with_topic, register_client};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol};

#[test]
fn protocol_fee_bps_change_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);

    let admin = Address::generate(&env);
    // initialize sets the admin for the contract
    client.initialize(&admin);

    // Change protocol fee bps
    assert!(client.set_protocol_fee_bps(&100u32));

    // Ensure an event with the protocol_fee_bps topic exists
    assert!(has_event_with_topic(
        &env,
        &Symbol::new(&env, "protocol_fee_bps")
    ));
}

#[test]
fn admin_propose_and_accept_emit_events() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let next_admin = Address::generate(&env);
    client.propose_governance_admin(&next_admin);

    // Accept requires the proposed admin to authorize — mock_all_auths covers this.
    client.accept_governance_admin();

    // Ensure admin-topic events exist (proposed / accepted)
    assert!(has_event_with_topic(&env, &soroban_sdk::symbol_short!("admin")));
}
