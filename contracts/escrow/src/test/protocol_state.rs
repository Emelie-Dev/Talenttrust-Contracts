//! Tests for the `get_protocol_state` read-only view.
//!
//! Covers:
//!  * Uninitialized state returns sensible defaults
//!  * Initialized state reflects all stored values
//!  * Paused / emergency flags are captured
//!  * Settlement token binding is reflected
//!  * Governed parameters are captured
//!  * Protocol fees accumulator is captured
//!  * next_contract_id increments are reflected
//!  * Boundary: zero fees, zero accumulated fees

use super::{MILESTONE_ONE, MILESTONE_THREE, MILESTONE_TWO};
use crate::{ReadinessChecklist, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, Address, Env};

use super::EscrowFixtureBuilder;

#[test]
fn uninitialized_state_returns_defaults() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(crate::Escrow, ());
    let escrow = crate::EscrowClient::new(&env, &id);
    let state = escrow.get_protocol_state();

    assert!(!state.initialized);
    assert!(state.admin.is_none());
    assert!(!state.paused);
    assert!(!state.emergency);
    assert!(state.settlement_token.is_none());
    assert_eq!(state.next_contract_id, 1);
    assert_eq!(state.protocol_fee_bps, 0);
    assert_eq!(state.accumulated_protocol_fees, 0);
    assert!(state.max_escrow_total_stroops.is_none());
    assert_eq!(
        state.readiness,
        ReadinessChecklist {
            initialized: false,
            governed_params_set: false,
            emergency_controls_enabled: false,
        }
    );
}

#[test]
fn initialized_state_reflects_admin() {
    let f = EscrowFixtureBuilder::new().funded().build();
    let state = f.escrow().get_protocol_state();

    assert!(state.initialized);
    assert!(state.admin.is_some());
    assert_eq!(state.admin.unwrap(), f.admin);
    assert_eq!(state.next_contract_id, 2);
}

#[test]
fn paused_flag_is_captured() {
    let f = EscrowFixtureBuilder::new().funded().build();
    f.escrow().pause();

    let state = f.escrow().get_protocol_state();
    assert!(state.paused);
}

#[test]
fn emergency_flag_is_captured() {
    let f = EscrowFixtureBuilder::new().funded().build();
    f.escrow().activate_emergency_pause();

    let state = f.escrow().get_protocol_state();
    assert!(state.emergency);
    assert!(state.paused);
}

#[test]
fn settlement_token_is_captured() {
    let f = EscrowFixtureBuilder::new().with_settlement_token().build();
    let state = f.escrow().get_protocol_state();

    assert!(state.settlement_token.is_some());
    assert_eq!(state.settlement_token.unwrap(), f.settlement_token.unwrap());
}

#[test]
fn governed_parameters_are_captured() {
    let f = EscrowFixtureBuilder::new().funded().build();
    f.escrow()
        .set_governed_params(&f.admin, &250, &1_000_000_000_000);

    let state = f.escrow().get_protocol_state();
    assert_eq!(state.protocol_fee_bps, 250);
    assert_eq!(state.max_escrow_total_stroops, Some(1_000_000_000_000));
}

#[test]
fn protocol_fee_bps_is_captured() {
    let f = EscrowFixtureBuilder::new().funded().build();
    f.escrow().set_protocol_fee_bps(&150);

    let state = f.escrow().get_protocol_state();
    assert_eq!(state.protocol_fee_bps, 150);
}

#[test]
fn next_contract_id_increments() {
    let f = EscrowFixtureBuilder::new().funded().build();

    let state1 = f.escrow().get_protocol_state();
    assert_eq!(state1.next_contract_id, 2);

    let client2 = Address::generate(&f.env);
    let freelancer2 = Address::generate(&f.env);
    let milestones = soroban_sdk::vec![&f.env, MILESTONE_ONE, MILESTONE_TWO, MILESTONE_THREE,];
    f.escrow().create_contract(
        &client2,
        &freelancer2,
        &None::<Address>,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    let state2 = f.escrow().get_protocol_state();
    assert_eq!(state2.next_contract_id, 3);
}

#[test]
fn accumulated_fees_are_captured() {
    let f = EscrowFixtureBuilder::new().funded().build();
    f.escrow()
        .approve_milestone_release(&f.escrow_id, &f.client, &0);
    f.escrow().release_milestone(&f.escrow_id, &f.client, &0);

    let state = f.escrow().get_protocol_state();
    assert!(state.accumulated_protocol_fees >= 0);
}

#[test]
fn readiness_checklist_is_captured() {
    let f = EscrowFixtureBuilder::new().funded().build();
    let state = f.escrow().get_protocol_state();

    assert!(state.readiness.initialized);
    assert!(!state.readiness.governed_params_set);
    assert!(!state.readiness.emergency_controls_enabled);

    f.escrow()
        .set_governed_params(&f.admin, &100, &500_000_000_000);

    let state2 = f.escrow().get_protocol_state();
    assert!(state2.readiness.governed_params_set);
}

#[test]
fn protocol_state_matches_individual_getters() {
    let f = EscrowFixtureBuilder::new().with_settlement_token().build();
    f.escrow().set_protocol_fee_bps(&100);
    f.escrow()
        .set_governed_params(&f.admin, &100, &800_000_000_000);

    let state = f.escrow().get_protocol_state();

    assert!(state.initialized);
    assert_eq!(state.admin, f.escrow().get_admin());
    assert_eq!(state.paused, f.escrow().is_paused());
    assert_eq!(state.emergency, f.escrow().is_emergency());
    assert_eq!(state.settlement_token, f.escrow().get_settlement_token());
    assert_eq!(state.next_contract_id, f.escrow().get_next_contract_id());
    assert_eq!(state.protocol_fee_bps, f.escrow().get_protocol_fee_bps());
    assert_eq!(
        state.accumulated_protocol_fees,
        f.escrow().get_accumulated_protocol_fees()
    );
    let governed = f.escrow().get_governed_parameters();
    assert_eq!(
        state.max_escrow_total_stroops,
        governed.map(|g| g.max_escrow_total_stroops)
    );
    assert_eq!(state.readiness, f.escrow().get_mainnet_readiness_info());
}

#[test]
fn zero_boundary_values() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = super::register_client(&env);
    let state = escrow.get_protocol_state();

    assert_eq!(state.protocol_fee_bps, 0);
    assert_eq!(state.accumulated_protocol_fees, 0);
    assert_eq!(state.next_contract_id, 1);
}

#[test]
fn unpause_reflected_in_state() {
    let f = EscrowFixtureBuilder::new().funded().build();
    f.escrow().pause();
    assert!(f.escrow().get_protocol_state().paused);

    f.escrow().unpause();
    assert!(!f.escrow().get_protocol_state().paused);
}
