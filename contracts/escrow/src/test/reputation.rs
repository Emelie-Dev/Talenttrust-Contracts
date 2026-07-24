use super::{complete_contract, create_contract, default_milestones, register_client, total_milestone_amount};
use crate::{Contract, ContractStatus, DataKey, EscrowError};
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

fn valid_comment(env: &Env) -> String {
    String::from_str(env, "Great job!")
}

// ---------------------------------------------------------------------------
// issue_reputation — negative paths
// ---------------------------------------------------------------------------

#[test]
fn issue_reputation_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);
    let unauthorized = Address::generate(&env);

    let result =
        client.try_issue_reputation(&contract_id, &unauthorized, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

#[test]
fn issue_reputation_rejects_non_completed_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn issue_reputation_rejects_invalid_rating_bounds() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    let result_low =
        client.try_issue_reputation(&contract_id, &client_addr, &0, &valid_comment(&env));
    super::assert_contract_error(result_low, EscrowError::InvalidRating);

    let result_high =
        client.try_issue_reputation(&contract_id, &client_addr, &6, &valid_comment(&env));
    super::assert_contract_error(result_high, EscrowError::InvalidRating);
}

#[test]
fn issue_reputation_rejects_empty_comment() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    let empty_comment = String::from_str(&env, "");
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &empty_comment);
    super::assert_contract_error(result, EscrowError::EmptyComment);
}

#[test]
fn issue_reputation_rejects_comment_too_long() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    let long_str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let long_comment = String::from_str(&env, long_str);
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &long_comment);
    super::assert_contract_error(result, EscrowError::CommentTooLong);
}

#[test]
fn issue_reputation_rejects_duplicate_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert!(client.issue_reputation(
        &contract_id,
        &client_addr,
        &5,
        &valid_comment(&env)
    ));
    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ReputationAlreadyIssued);
}

#[test]
fn issue_reputation_rejects_self_rating_when_client_equals_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    env.as_contract(&client.address, || {
        let key = DataKey::Contract(contract_id);
        let mut contract: Contract = env.storage().persistent().get(&key).unwrap();
        contract.freelancer = client_addr.clone();
        env.storage().persistent().set(&key, &contract);
    });

    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::SelfRating);
}

// ---------------------------------------------------------------------------
// Pending-credit lifecycle — positive paths
// ---------------------------------------------------------------------------

#[test]
fn pending_credit_incremented_on_contract_completion() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, freelancer_addr, _contract_id) = complete_contract(&env, &client);

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
}

#[test]
fn pending_credit_consumed_on_reputation_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
    assert!(client.issue_reputation(
        &contract_id,
        &client_addr,
        &5,
        &valid_comment(&env)
    ));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);
}

#[test]
fn pending_credit_lifecycle_multiple_contracts_same_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let freelancer_addr = Address::generate(&env);

    // Complete 3 contracts for the same freelancer
    for _ in 0..3 {
        let milestones = default_milestones(&env);
        let client_addr = Address::generate(&env);
        let id = client.create_contract(
            &client_addr,
            &freelancer_addr,
            &None,
            &milestones,
            &crate::ReleaseAuthorization::ClientOnly,
        );
        let total = total_milestone_amount();
        client.deposit_funds(&id, &client_addr, &total);
        client.approve_milestone_release(&id, &client_addr, &0);
        client.release_milestone(&id, &client_addr, &0);
        client.approve_milestone_release(&id, &client_addr, &1);
        client.release_milestone(&id, &client_addr, &1);
        client.approve_milestone_release(&id, &client_addr, &2);
        client.release_milestone(&id, &client_addr, &2);
    }

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 3);
}

#[test]
fn pending_credits_consumed_one_per_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let freelancer_addr = Address::generate(&env);
    let mut contract_ids = Vec::new(&env);
    let mut client_addresses = Vec::new(&env);

    // Complete 2 contracts for the same freelancer
    for _ in 0..2 {
        let milestones = default_milestones(&env);
        let client_addr = Address::generate(&env);
        let id = client.create_contract(
            &client_addr,
            &freelancer_addr,
            &None,
            &milestones,
            &crate::ReleaseAuthorization::ClientOnly,
        );
        let total = total_milestone_amount();
        client.deposit_funds(&id, &client_addr, &total);
        client.approve_milestone_release(&id, &client_addr, &0);
        client.release_milestone(&id, &client_addr, &0);
        client.approve_milestone_release(&id, &client_addr, &1);
        client.release_milestone(&id, &client_addr, &1);
        client.approve_milestone_release(&id, &client_addr, &2);
        client.release_milestone(&id, &client_addr, &2);
        contract_ids.push_back(id);
        client_addresses.push_back(client_addr);
    }

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 2);

    // Issue reputation for first contract — one credit consumed
    assert!(client.issue_reputation(
        &contract_ids.get(0).unwrap(),
        &client_addresses.get(0).unwrap(),
        &4,
        &valid_comment(&env)
    ));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);

    // Issue reputation for second contract — second credit consumed
    assert!(client.issue_reputation(
        &contract_ids.get(1).unwrap(),
        &client_addresses.get(1).unwrap(),
        &5,
        &valid_comment(&env)
    ));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);
}

#[test]
fn pending_credits_never_incremented_for_refunded_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &client);

    let milestones = client.get_milestones(&contract_id);
    let indices = vec![&env, 0u32, 1u32, 2u32];
    client.refund_unreleased_milestones(&contract_id, &indices);

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Refunded);
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);
}

#[test]
fn pending_credit_incremented_on_mixed_refund_and_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &client);

    let total = total_milestone_amount();
    client.deposit_funds(&contract_id, &client_addr, &total);
    client.approve_milestone_release(&contract_id, &client_addr, &0);
    client.release_milestone(&contract_id, &client_addr, &0);

    // Refund remaining milestones (1 and 2)
    let indices = vec![&env, 1u32, 2u32];
    client.refund_unreleased_milestones(&contract_id, &indices);

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
}

#[test]
fn issue_reputation_succeeds_for_distinct_client_and_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert!(client.issue_reputation(
        &contract_id,
        &client_addr,
        &5,
        &valid_comment(&env)
    ));
}

#[test]
fn issue_reputation_updates_reputation_record() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
    assert!(client.issue_reputation(
        &contract_id,
        &client_addr,
        &5,
        &valid_comment(&env)
    ));

    let reputation = client
        .get_reputation(&freelancer_addr)
        .expect("expected reputation record");
    assert_eq!(reputation.completed_contracts, 1);
    assert_eq!(reputation.total_rating, 5);
    assert_eq!(reputation.last_rating, 5);
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);
}

// ---------------------------------------------------------------------------
// Pending-credit guard — issue_reputation panics when pending is 0
// ---------------------------------------------------------------------------

#[test]
fn issue_reputation_panics_when_pending_credits_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = complete_contract(&env, &client);

    // Consume the pending credit
    assert!(client.issue_reputation(
        &contract_id,
        &client_addr,
        &5,
        &valid_comment(&env)
    ));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);

    // Reset reputation_issued to false so we can bypass the duplicate guard
    // and reach the pending-credit guard. This simulates a scenario where
    // the storage invariant is violated (pending == 0 but issue_reputation
    // is called for a contract that hasn't had reputation issued yet).
    env.as_contract(&client.address, || {
        let key = DataKey::Contract(contract_id);
        let mut contract: Contract = env.storage().persistent().get(&key).unwrap();
        contract.reputation_issued = false;
        env.storage().persistent().set(&key, &contract);
    });

    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::InvalidState);
}

// ---------------------------------------------------------------------------
// get_average_rating tests
// ---------------------------------------------------------------------------

#[test]
fn get_average_rating_returns_none_for_unknown_address() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let unknown = Address::generate(&env);
    assert!(client.get_average_rating(&unknown).is_none());
}

#[test]
fn get_average_rating_single_rating_returns_scaled_value() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = complete_contract(&env, &client);

    client.issue_reputation(
        &contract_id,
        &client_addr,
        &4,
        &valid_comment(&env),
    );

    assert_eq!(client.get_average_rating(&freelancer_addr), Some(40_000));
}

#[test]
fn get_average_rating_multiple_ratings_returns_correct_scaled_average() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr1, freelancer_addr, contract_id1) = complete_contract(&env, &client);
    client.issue_reputation(
        &contract_id1,
        &client_addr1,
        &3,
        &valid_comment(&env),
    );

    let client_addr2 = Address::generate(&env);
    let milestones = default_milestones(&env);
    let contract_id2 = client.create_contract(
        &client_addr2,
        &freelancer_addr,
        &None,
        &milestones,
        &crate::ReleaseAuthorization::ClientOnly,
    );
    let total = total_milestone_amount();
    client.deposit_funds(&contract_id2, &client_addr2, &total);
    client.approve_milestone_release(&contract_id2, &client_addr2, &0);
    client.release_milestone(&contract_id2, &client_addr2, &0);
    client.approve_milestone_release(&contract_id2, &client_addr2, &1);
    client.release_milestone(&contract_id2, &client_addr2, &1);
    client.approve_milestone_release(&contract_id2, &client_addr2, &2);
    client.release_milestone(&contract_id2, &client_addr2, &2);
    client.issue_reputation(
        &contract_id2,
        &client_addr2,
        &5,
        &valid_comment(&env),
    );

    assert_eq!(client.get_average_rating(&freelancer_addr), Some(40_000));
}

#[test]
fn get_average_rating_fractional_average_is_preserved() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr1, freelancer_addr, contract_id1) = complete_contract(&env, &client);
    client.issue_reputation(
        &contract_id1,
        &client_addr1,
        &1,
        &valid_comment(&env),
    );

    let client_addr2 = Address::generate(&env);
    let milestones = default_milestones(&env);
    let contract_id2 = client.create_contract(
        &client_addr2,
        &freelancer_addr,
        &None,
        &milestones,
        &crate::ReleaseAuthorization::ClientOnly,
    );
    let total = total_milestone_amount();
    client.deposit_funds(&contract_id2, &client_addr2, &total);
    client.approve_milestone_release(&contract_id2, &client_addr2, &0);
    client.release_milestone(&contract_id2, &client_addr2, &0);
    client.approve_milestone_release(&contract_id2, &client_addr2, &1);
    client.release_milestone(&contract_id2, &client_addr2, &1);
    client.approve_milestone_release(&contract_id2, &client_addr2, &2);
    client.release_milestone(&contract_id2, &client_addr2, &2);
    client.issue_reputation(
        &contract_id2,
        &client_addr2,
        &2,
        &valid_comment(&env),
    );

    assert_eq!(client.get_average_rating(&freelancer_addr), Some(15_000));
}
