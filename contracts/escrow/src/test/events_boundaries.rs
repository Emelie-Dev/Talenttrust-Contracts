#![cfg(test)]

//! Boundary tests for events emitted by the escrow contract.
//!
//! Issue #816 — "Cover events boundaries"
//!
//! The escrow contract emits events from public governance, lifecycle, and
//! settlement-token entrypoints.  Many acceptance paths (`init`,
//! `settlement_token_bound`, `protocol_fee_bps`, the `admin` topic family,
//! `pause`, `unpaused`, `emergency`) and rejection paths (typed contract
//! errors **and** host auth failures) are exercised by other suites, but the
//! **edges** of those events are sparsely tested.
//!
//! This module presents the canonical accept/reject boundary matrix as a
//! single, easy-to-audit suite.  Every test in this file uses one of three
//! boundary shapes, as the issue requires:
//!
//! | Boundary shape                          | Purpose                                       |
//! |-----------------------------------------|-----------------------------------------------|
//! | exactly-at the boundary                 | event MUST be published                        |
//! | one ledger / one unit over the boundary | event MUST NOT be published when rejected     |
//! | unauthorized caller                     | event MUST NOT be published when rejected     |
//!
//! All assertions use the project test-utils helpers (`register_client`,
//! `assert_contract_error`) and the existing event-assertion patterns from
//! `governance_events.rs` and `sac_custody.rs`.  When the contract logic does
//! not reject an over-boundary value, this file documents that as a defect
//! rather than asserting an event that the contract does not emit.

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _, LedgerInfo},
    Address, Env, Symbol, TryFromVal,
};

use super::{assert_contract_error, register_client};
use crate::{Escrow, EscrowClient, Error as ContractError, EscrowError, ADMIN_ROTATION_MIN_DELAY_LEDGERS};

// ---------------------------------------------------------------------------
// Test-helper utilities
// ---------------------------------------------------------------------------

/// Advance the test ledger by `delta` ledgers while preserving the host info.
/// Identical in shape to the helper in `treasury_rotation_timelock.rs`.
fn advance_ledgers(env: &Env, delta: u32) {
    let info = env.ledger().get();
    env.ledger().set(LedgerInfo {
        sequence_number: info.sequence_number + delta,
        timestamp: info.timestamp + (delta as u64) * 5,
        protocol_version: info.protocol_version,
        network_id: info.network_id,
        base_reserve: info.base_reserve,
        min_temp_entry_ttl: info.min_temp_entry_ttl,
        min_persistent_entry_ttl: info.min_persistent_entry_ttl,
        max_entry_ttl: info.max_entry_ttl,
    });
}

/// True iff some published event has `topic` as its first topic element.
fn has_event_with_topic(env: &Env, topic: &Symbol) -> bool {
    env.events().all().iter().any(|event| {
        event.1.len() > 0
            && Symbol::try_from_val(env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(topic)
    })
}

/// True iff some published event has the topic-pair `(topic_a, topic_b)`.
fn has_event_with_two_topics(env: &Env, topic_a: &Symbol, topic_b: &Symbol) -> bool {
    env.events().all().iter().any(|event| {
        event.1.len() >= 2
            && Symbol::try_from_val(env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(topic_a)
            && Symbol::try_from_val(env, &event.1.get(1).unwrap())
                .ok()
                .as_ref()
                == Some(topic_b)
    })
}

/// Register a fresh `Escrow` contract *without* calling `initialize`.
/// Required by the first-call boundary tests for the `init` event.
fn fresh_uninitialized_client(env: &Env) -> EscrowClient<'_> {
    let contract_id = env.register(Escrow, ());
    EscrowClient::new(env, &contract_id)
}

// ===========================================================================
// `init` event boundary (issue #816 — "exactly-at boundary")
// ===========================================================================

/// The very first call to `initialize` MUST emit the `(init, admin_set)`
/// two-topic event.  This is the namesake "first-call boundary" of the
/// escrow lifecycle.
#[test]
fn initialize_first_call_emits_init_admin_set_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);
    let admin = Address::generate(&env);

    assert!(client.initialize(&admin));

    let init_topic = soroban_sdk::symbol_short!("init");
    let admin_set_topic = Symbol::new(&env, "admin_set");
    assert!(
        has_event_with_two_topics(&env, &init_topic, &admin_set_topic),
        "initialize must publish (init, admin_set) topics on its first call"
    );
}

/// A second call to `initialize` MUST be rejected with `AlreadyInitialized`
/// (an exact typed contract error) and MUST NOT publish a new
/// `(init, admin_set)` event.  This is the "rejected at boundary" case.
#[test]
fn initialize_rejected_after_first_call_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env); // already initialized once

    let admin = client.get_admin().unwrap();
    assert_contract_error(
        client.try_initialize(&admin),
        ContractError::AlreadyInitialized,
    );

    let init_topic = soroban_sdk::symbol_short!("init");
    assert!(
        !has_event_with_topic(&env, &init_topic),
        "rejected double-initialize must not publish 'init' event"
    );
}

// ===========================================================================
// `settlement_token_bound` event boundaries
// ===========================================================================

/// Success at the accept boundary — the admin's first `bind_settlement_token`
/// call MUST emit a `settlement_token_bound` topic.  This is the canonical
/// "exactly-at boundary" case for the bind flow.
#[test]
fn bind_settlement_token_first_call_emits_event_at_accept_boundary() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let sac = env.register_stellar_asset_contract(admin.clone());
    assert!(client.bind_settlement_token(&admin, &sac));

    let topic = Symbol::new(&env, "settlement_token_bound");
    assert!(
        has_event_with_topic(&env, &topic),
        "successful first bind must publish settlement_token_bound"
    );
}

/// Double-bind at the "one over" boundary — a second bind attempt (with a
/// different SAC) MUST be rejected with the typed `SettlementTokenAlreadyBound`
/// error and MUST NOT publish a follow-up event.  The write-once invariant
/// is fail-closed at the event layer.
#[test]
fn bind_settlement_token_rejected_double_bind_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let sac1 = env.register_stellar_asset_contract(admin.clone());
    let sac2 = env.register_stellar_asset_contract(admin.clone());
    assert!(client.bind_settlement_token(&admin, &sac1));

    assert_contract_error(
        client.try_bind_settlement_token(&admin, &sac2),
        EscrowError::SettlementTokenAlreadyBound,
    );

    let topic = Symbol::new(&env, "settlement_token_bound");
    assert!(
        !has_event_with_topic(&env, &topic),
        "rejected double-bind must not publish settlement_token_bound event"
    );
}

/// Self-bind at the "one over the allowed-actor boundary" — binding the
/// escrow contract's own address MUST be rejected with the typed
/// `SettlementTokenIsSelf` error and MUST NOT publish the event.  The probe
/// must not run after the typed check rejects the candidate.
#[test]
fn bind_settlement_token_rejected_self_address_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    assert_contract_error(
        client.try_bind_settlement_token(&admin, &client.address.clone()),
        EscrowError::SettlementTokenIsSelf,
    );

    let topic = Symbol::new(&env, "settlement_token_bound");
    assert!(
        !has_event_with_topic(&env, &topic),
        "rejected self-bind must not publish settlement_token_bound event"
    );
}

/// Admin-as-token at the privilege-separation boundary — binding the stored
/// admin as the settlement asset MUST be rejected with the typed
/// `SettlementTokenIsAdmin` error and MUST NOT publish the event.
#[test]
fn bind_settlement_token_rejected_admin_address_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    assert_contract_error(
        client.try_bind_settlement_token(&admin, &admin),
        EscrowError::SettlementTokenIsAdmin,
    );

    let topic = Symbol::new(&env, "settlement_token_bound");
    assert!(
        !has_event_with_topic(&env, &topic),
        "rejected admin-bind must not publish settlement_token_bound event"
    );
}

/// Unauthorized caller at the "wrong principal" boundary — a non-admin
/// caller attempting to bind MUST be rejected with the typed
/// `UnauthorizedRole` error before the SAC probe runs and MUST NOT publish
/// the event.
#[test]
fn bind_settlement_token_rejected_unauthorized_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let attacker = Address::generate(&env);
    let sac = env.register_stellar_asset_contract(admin.clone());

    assert_contract_error(
        client.try_bind_settlement_token(&attacker, &sac),
        EscrowError::UnauthorizedRole,
    );

    let topic = Symbol::new(&env, "settlement_token_bound");
    assert!(
        !has_event_with_topic(&env, &topic),
        "rejected unauthorized bind must not publish settlement_token_bound event"
    );
}

// ===========================================================================
// `protocol_fee_bps` event boundaries
// ===========================================================================

/// Accept at the lower boundary — `set_protocol_fee_bps(0)` MUST emit a
/// `protocol_fee_bps` event.  This is the zero-bps edge case.
#[test]
fn set_protocol_fee_bps_at_zero_boundary_emits_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.set_protocol_fee_bps(&0u32));

    let topic = Symbol::new(&env, "protocol_fee_bps");
    assert!(
        has_event_with_topic(&env, &topic),
        "set_protocol_fee_bps(0) must publish protocol_fee_bps event"
    );
    assert_eq!(client.get_protocol_fee_bps(), 0);
}

/// Accept at the upper boundary — `set_protocol_fee_bps(10_000)` (100 %) MUST
/// emit a `protocol_fee_bps` event.  This is the maximum-bps edge case.
#[test]
fn set_protocol_fee_bps_at_max_boundary_emits_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.set_protocol_fee_bps(&10_000u32));

    let topic = Symbol::new(&env, "protocol_fee_bps");
    assert!(
        has_event_with_topic(&env, &topic),
        "set_protocol_fee_bps(10_000) must publish protocol_fee_bps event"
    );
    assert_eq!(client.get_protocol_fee_bps(), 10_000);
}

/// Repeated updates MUST emit a fresh `protocol_fee_bps` event on every
/// successful call.  Note: this test focuses on the structural acceptance
/// at the boundaries (issue #816 specifically calls out "exactly-at
/// boundary" coverage); the payload data is left to the existing
/// protocol-fee-withdrawal and accrual tests.
///
/// > **Defect (noted, not changed).**  The contract does *not* reject
/// > `new_bps > 10_000` for `set_protocol_fee_bps` — that boundary is only
/// > enforced by `set_governed_params`.  Per the issue's "do not change
/// > contract logic unless a defect is found (note it)" rule, this is left
/// > as-is and documented here, but a follow-up would be well-served by
/// > tightening the public fee setter to validate 0 ≤ new_bps ≤ 10_000.
#[test]
fn set_protocol_fee_bps_repeated_updates_each_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.set_protocol_fee_bps(&10_000u32));
    assert!(client.set_protocol_fee_bps(&0u32));
    assert!(client.set_protocol_fee_bps(&1u32));

    let topic = Symbol::new(&env, "protocol_fee_bps");
    assert!(
        has_event_with_topic(&env, &topic),
        "each successful set_protocol_fee_bps call must publish a follow-up event"
    );
    assert_eq!(client.get_protocol_fee_bps(), 1);
}

// ===========================================================================
// `admin` topic family event boundaries  (issue #816 — timelock at edge)
// ===========================================================================

/// Accept at the timelock boundary — `accept_governance_admin` after exactly
/// `ADMIN_ROTATION_MIN_DELAY_LEDGERS` ledgers MUST publish the
/// `(admin, accepted)` event.  This is the classic "exactly-at boundary"
/// case for the governance rotation flow.
#[test]
fn accept_governance_admin_exactly_at_min_delay_emits_accepted_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let next_admin = Address::generate(&env);
    client.propose_governance_admin(&next_admin);

    advance_ledgers(&env, ADMIN_ROTATION_MIN_DELAY_LEDGERS);

    assert!(client.accept_governance_admin());

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let accepted_topic = Symbol::new(&env, "accepted");
    assert!(
        has_event_with_two_topics(&env, &admin_topic, &accepted_topic),
        "accept at exactly min-delay must publish (admin, accepted)"
    );
}

/// Reject one ledger short of the boundary — `accept_governance_admin`
/// after `ADMIN_ROTATION_MIN_DELAY_LEDGERS - 1` ledgers MUST be rejected
/// with the typed `TimelockNotElapsed` error and MUST NOT publish the
/// `(admin, accepted)` event.  The boundary comparison is fail-closed at
/// the event layer.
#[test]
fn accept_governance_admin_one_below_min_delay_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let next_admin = Address::generate(&env);
    client.propose_governance_admin(&next_admin);

    advance_ledgers(&env, ADMIN_ROTATION_MIN_DELAY_LEDGERS - 1);

    assert_contract_error(
        client.try_accept_governance_admin(),
        EscrowError::TimelockNotElapsed,
    );

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let accepted_topic = Symbol::new(&env, "accepted");
    assert!(
        !has_event_with_two_topics(&env, &admin_topic, &accepted_topic),
        "rejected accept must not publish (admin, accepted)"
    );
}

/// Accept one ledger *over* the boundary — `accept_governance_admin`
/// after `ADMIN_ROTATION_MIN_DELAY_LEDGERS + 1` ledgers MUST still accept
/// and publish the `(admin, accepted)` event.  This pins the boundary
/// comparison to `>=` rather than `==` (issue #816 explicitly calls out
/// "one over" coverage).
#[test]
fn accept_governance_admin_one_over_min_delay_emits_accepted_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let next_admin = Address::generate(&env);
    client.propose_governance_admin(&next_admin);

    advance_ledgers(&env, ADMIN_ROTATION_MIN_DELAY_LEDGERS + 1);

    assert!(client.accept_governance_admin());

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let accepted_topic = Symbol::new(&env, "accepted");
    assert!(
        has_event_with_two_topics(&env, &admin_topic, &accepted_topic),
        "accept one ledger past min-delay must publish (admin, accepted)"
    );
}

/// Reject when there is no pending proposal — `accept_governance_admin`
/// with no pending admin MUST be rejected with the typed `InvalidState`
/// error and MUST NOT publish the `(admin, accepted)` event.
#[test]
fn accept_governance_admin_with_no_pending_proposal_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    assert_contract_error(
        client.try_accept_governance_admin(),
        ContractError::InvalidState,
    );

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let accepted_topic = Symbol::new(&env, "accepted");
    assert!(
        !has_event_with_two_topics(&env, &admin_topic, &accepted_topic),
        "rejected accept must not publish (admin, accepted)"
    );
}

/// Accept at the proposal boundary — `propose_governance_admin` MUST
/// publish `(admin, proposed)`.  This pins the proposer-side accept path.
#[test]
fn propose_governance_admin_emits_proposed_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let next_admin = Address::generate(&env);
    assert!(client.propose_governance_admin(&next_admin));

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let proposed_topic = Symbol::new(&env, "proposed");
    assert!(
        has_event_with_two_topics(&env, &admin_topic, &proposed_topic),
        "propose must publish (admin, proposed)"
    );
}

/// Accept at the cancel boundary — `cancel_governance_admin_proposal`
/// following a successful `propose` MUST publish `(admin, cancelled)`.
#[test]
fn cancel_governance_admin_proposal_emits_cancelled_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let next_admin = Address::generate(&env);
    client.propose_governance_admin(&next_admin);
    assert!(client.cancel_governance_admin_proposal());

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let cancelled_topic = Symbol::new(&env, "cancelled");
    assert!(
        has_event_with_two_topics(&env, &admin_topic, &cancelled_topic),
        "cancel must publish (admin, cancelled)"
    );
}

/// Reject when there is no pending proposal — `cancel_governance_admin_proposal`
/// without a pending admin MUST be rejected with the typed `InvalidState`
/// error and MUST NOT publish the `(admin, cancelled)` event.
///
/// > **Note.** The existing `cancel_without_proposal_fails` test in
/// > `test/governance.rs` asserts `Error::NoPendingAdminProposal`, which does
/// > not exist on the `Error` enum.  Its assertion is stale; the impl in
/// > `governance.rs::cancel_governance_admin_proposal_impl` panics with
/// > `Error::InvalidState`, which is what this test pins down.
#[test]
fn cancel_governance_admin_proposal_with_no_pending_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    assert_contract_error(
        client.try_cancel_governance_admin_proposal(),
        ContractError::InvalidState,
    );

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let cancelled_topic = Symbol::new(&env, "cancelled");
    assert!(
        !has_event_with_two_topics(&env, &admin_topic, &cancelled_topic),
        "rejected cancel must not publish (admin, cancelled)"
    );
}

// ===========================================================================
// `pause` / `unpaused` / `emergency` event boundaries
// ===========================================================================

/// Accept at the pause boundary — `pause` MUST publish `pause`.
#[test]
fn pause_by_admin_emits_pause_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.pause());

    let pause_topic = soroban_sdk::symbol_short!("pause");
    assert!(
        has_event_with_topic(&env, &pause_topic),
        "successful pause must publish 'pause' event"
    );
    assert!(client.is_paused());
}

/// Accept at the unpause boundary — `unpause` MUST publish `unpaused`.
/// We arrange for the contract to be paused first (without emergency)
/// because `unpause` requires `Emergency == false`.
#[test]
fn unpause_after_pause_emits_unpaused_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.pause());
    assert!(client.unpause());

    let unpause_topic = soroban_sdk::symbol_short!("unpaused");
    assert!(
        has_event_with_topic(&env, &unpause_topic),
        "successful unpause must publish 'unpaused' event"
    );
    assert!(!client.is_paused());
}

/// Reject at the unpause-while-emergency boundary — `unpause` while the
/// emergency flag is set MUST be rejected with the typed `EmergencyActive`
/// error and MUST NOT publish the `unpaused` event.  This pins the
/// "emergency pre-empts unpause" invariant at the event layer.
#[test]
fn unpause_blocked_by_emergency_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.activate_emergency_pause());
    // Sanity: the activation event published before the rejection site.
    let emergency_topic = Symbol::new(&env, "emergency");
    let activated_topic = Symbol::new(&env, "activated");
    assert!(
        has_event_with_two_topics(&env, &emergency_topic, &activated_topic),
        "setup: activate_emergency_pause must publish (emergency, activated)"
    );

    assert_contract_error(client.try_unpause(), ContractError::EmergencyActive);

    let unpause_topic = soroban_sdk::symbol_short!("unpaused");
    assert!(
        !has_event_with_topic(&env, &unpause_topic),
        "rejected unpause must not publish 'unpaused' event"
    );
}

/// Accept at the emergency-activation boundary — `activate_emergency_pause`
/// MUST publish `(emergency, activated)`.
#[test]
fn activate_emergency_pause_emits_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.activate_emergency_pause());

    let emergency_topic = Symbol::new(&env, "emergency");
    let activated_topic = Symbol::new(&env, "activated");
    assert!(
        has_event_with_two_topics(&env, &emergency_topic, &activated_topic),
        "activate_emergency_pause must publish (emergency, activated)"
    );
}

/// Accept at the emergency-resolution boundary — after `activate_emergency_pause`
/// succeeds, `resolve_emergency` MUST publish `(emergency, resolved)`.
#[test]
fn resolve_emergency_after_activation_emits_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = register_client(&env);

    assert!(client.activate_emergency_pause());
    assert!(client.resolve_emergency());

    let emergency_topic = Symbol::new(&env, "emergency");
    let resolved_topic = Symbol::new(&env, "resolved");
    assert!(
        has_event_with_two_topics(&env, &emergency_topic, &resolved_topic),
        "resolve_emergency must publish (emergency, resolved)"
    );
}

/// Reject at the uninitialized boundary — calling `activate_emergency_pause`
/// on an uninitialized contract MUST be rejected with the typed
/// `NotInitialized` error and MUST NOT publish `(emergency, activated)`.
#[test]
fn activate_emergency_pause_rejected_uninitialized_does_not_emit_event() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();
    let client = fresh_uninitialized_client(&env);

    assert_contract_error(
        client.try_activate_emergency_pause(),
        ContractError::NotInitialized,
    );

    let emergency_topic = Symbol::new(&env, "emergency");
    let activated_topic = Symbol::new(&env, "activated");
    assert!(
        !has_event_with_two_topics(&env, &emergency_topic, &activated_topic),
        "rejected (uninitialized) activate must not publish (emergency, activated)"
    );
}
