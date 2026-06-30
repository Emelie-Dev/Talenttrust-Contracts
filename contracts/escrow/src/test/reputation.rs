use super::{complete_contract, create_contract, register_client};
use crate::{Contract, DataKey, EscrowError};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn valid_comment(env: &Env) -> String {
    String::from_str(env, "Great job!")
}

#[test]
fn issue_reputation_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);
    let unauthorized = Address::generate(&env);

    let result = client.try_issue_reputation(&contract_id, &unauthorized, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

#[test]
fn issue_reputation_rejects_non_completed_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn issue_reputation_rejects_invalid_rating_bounds() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    let result_low = client.try_issue_reputation(&contract_id, &client_addr, &0, &valid_comment(&env));
    super::assert_contract_error(result_low, EscrowError::InvalidRating);

    let result_high = client.try_issue_reputation(&contract_id, &client_addr, &6, &valid_comment(&env));
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

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    let result = client.try_issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));
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

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::SelfRating);
}

#[test]
fn issue_reputation_succeeds_for_distinct_client_and_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
}

#[test]
fn issue_reputation_updates_reputation_record_and_pending_credits() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));

    let reputation = client
        .get_reputation(&freelancer_addr)
        .expect("expected reputation record");
    assert_eq!(reputation.completed_contracts, 1);
    assert_eq!(reputation.total_rating, 5);
    assert_eq!(reputation.last_rating, 5);
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);
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

    client.issue_reputation(&contract_id, &client_addr, &freelancer_addr, &4);

    // 4 * 10_000 / 1 = 40_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(40_000));
}

#[test]
fn get_average_rating_multiple_ratings_returns_correct_scaled_average() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    // First contract: rating 3
    let (client_addr1, freelancer_addr, contract_id1) = complete_contract(&env, &client);
    client.issue_reputation(&contract_id1, &client_addr1, &freelancer_addr, &3);

    // Second contract: same freelancer, rating 5
    let client_addr2 = Address::generate(&env);
    let milestones = super::default_milestones(&env);
    let contract_id2 = client.create_contract(
        &client_addr2,
        &freelancer_addr,
        &None,
        &milestones,
        &crate::ReleaseAuthorization::ClientOnly,
    );
    let total = super::total_milestone_amount();
    client.deposit_funds(&contract_id2, &client_addr2, &total);
    client.approve_milestone_release(&contract_id2, &client_addr2, &0);
    client.release_milestone(&contract_id2, &client_addr2, &0);
    client.approve_milestone_release(&contract_id2, &client_addr2, &1);
    client.release_milestone(&contract_id2, &client_addr2, &1);
    client.approve_milestone_release(&contract_id2, &client_addr2, &2);
    client.release_milestone(&contract_id2, &client_addr2, &2);
    client.issue_reputation(&contract_id2, &client_addr2, &freelancer_addr, &5);

    // total_rating=8, completed_contracts=2 → 8 * 10_000 / 2 = 40_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(40_000));
}

#[test]
fn get_average_rating_fractional_average_is_preserved() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    // First contract: rating 1
    let (client_addr1, freelancer_addr, contract_id1) = complete_contract(&env, &client);
    client.issue_reputation(&contract_id1, &client_addr1, &freelancer_addr, &1);

    // Second contract: rating 2
    let client_addr2 = Address::generate(&env);
    let milestones = super::default_milestones(&env);
    let contract_id2 = client.create_contract(
        &client_addr2,
        &freelancer_addr,
        &None,
        &milestones,
        &crate::ReleaseAuthorization::ClientOnly,
    );
    let total = super::total_milestone_amount();
    client.deposit_funds(&contract_id2, &client_addr2, &total);
    client.approve_milestone_release(&contract_id2, &client_addr2, &0);
    client.release_milestone(&contract_id2, &client_addr2, &0);
    client.approve_milestone_release(&contract_id2, &client_addr2, &1);
    client.release_milestone(&contract_id2, &client_addr2, &1);
    client.approve_milestone_release(&contract_id2, &client_addr2, &2);
    client.release_milestone(&contract_id2, &client_addr2, &2);
    client.issue_reputation(&contract_id2, &client_addr2, &freelancer_addr, &2);

    // total_rating=3, completed_contracts=2 → 3 * 10_000 / 2 = 15_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(15_000));
}
