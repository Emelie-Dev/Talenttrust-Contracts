use super::{has_event_with_topic, has_event_with_topics, register_client};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol};

#[test]
fn admin_transfer_propose_and_accept_happy_path() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let next_admin = Address::generate(&env);
    client.initialize(&admin);

    assert!(client.propose_governance_admin(&next_admin));
    assert_eq!(
        client.get_pending_governance_admin(),
        Some(next_admin.clone())
    );

    assert!(client.accept_governance_admin());
    assert_eq!(client.get_governance_admin(), Some(next_admin));
    assert_eq!(client.get_pending_governance_admin(), None);
}

#[test]
fn propose_self_as_admin_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let result = client.try_propose_governance_admin(&admin);
    super::assert_contract_error(result, crate::Error::CannotProposeSelf);

    // Pending admin should still be None
    assert_eq!(client.get_pending_governance_admin(), None);
}

#[test]
fn propose_overwrites_pending_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let first_pending = Address::generate(&env);
    let second_pending = Address::generate(&env);
    client.initialize(&admin);

    assert!(client.propose_governance_admin(&first_pending));
    assert_eq!(
        client.get_pending_governance_admin(),
        Some(first_pending.clone())
    );

    // Re-proposing should overwrite without error
    assert!(client.propose_governance_admin(&second_pending));
    assert_eq!(
        client.get_pending_governance_admin(),
        Some(second_pending.clone())
    );
}

#[test]
fn cancel_proposal_clears_pending_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let proposed = Address::generate(&env);
    client.initialize(&admin);

    assert!(client.propose_governance_admin(&proposed));
    assert_eq!(
        client.get_pending_governance_admin(),
        Some(proposed.clone())
    );

    assert!(client.cancel_governance_admin_proposal());
    assert_eq!(client.get_pending_governance_admin(), None);
}

#[test]
fn cancel_without_proposal_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    client.initialize(&admin);

    let result = client.try_cancel_governance_admin_proposal();
    super::assert_contract_error(result, crate::Error::NoPendingAdminProposal);
}

#[test]
fn accept_after_cancel_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let proposed = Address::generate(&env);
    client.initialize(&admin);

    assert!(client.propose_governance_admin(&proposed));
    assert!(client.cancel_governance_admin_proposal());

    let result = client.try_accept_governance_admin();
    super::assert_contract_error(result, crate::Error::InvalidState);
}

#[test]
fn propose_not_initialized_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let proposed = Address::generate(&env);
    let result = client.try_propose_governance_admin(&proposed);
    super::assert_contract_error(result, crate::Error::NotInitialized);
}

#[test]
fn accept_not_initialized_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let result = client.try_accept_governance_admin();
    super::assert_contract_error(result, crate::Error::NotInitialized);
}

#[test]
fn cancel_not_initialized_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let result = client.try_cancel_governance_admin_proposal();
    super::assert_contract_error(result, crate::Error::NotInitialized);
}

#[test]
fn propose_then_cancel_then_new_propose_then_accept() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let first_proposed = Address::generate(&env);
    let second_proposed = Address::generate(&env);
    client.initialize(&admin);

    // Propose first candidate
    assert!(client.propose_governance_admin(&first_proposed));
    assert_eq!(
        client.get_pending_governance_admin(),
        Some(first_proposed.clone())
    );

    // Cancel
    assert!(client.cancel_governance_admin_proposal());
    assert_eq!(client.get_pending_governance_admin(), None);

    // Propose second candidate
    assert!(client.propose_governance_admin(&second_proposed));
    assert_eq!(
        client.get_pending_governance_admin(),
        Some(second_proposed.clone())
    );

    // Accept moves second candidate to admin
    assert!(client.accept_governance_admin());
    assert_eq!(client.get_governance_admin(), Some(second_proposed));
    assert_eq!(client.get_pending_governance_admin(), None);
}

#[test]
fn cancel_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let proposed = Address::generate(&env);
    client.initialize(&admin);

    client.propose_governance_admin(&proposed);
    client.cancel_governance_admin_proposal();

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let cancelled_topic = soroban_sdk::Symbol::new(&env, "cancelled");
    assert!(
        has_event_with_topics(&env, &[admin_topic, cancelled_topic]),
        "cancel event should be emitted"
    );
}

#[test]
fn propose_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let admin = Address::generate(&env);
    let proposed = Address::generate(&env);
    client.initialize(&admin);

    client.propose_governance_admin(&proposed);

    let admin_topic = soroban_sdk::symbol_short!("admin");
    let proposed_topic = soroban_sdk::Symbol::new(&env, "proposed");
    assert!(
        has_event_with_topics(&env, &[admin_topic, proposed_topic]),
        "propose event should be emitted"
    );
}
