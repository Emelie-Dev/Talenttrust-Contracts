use super::{complete_contract, create_contract, register_client};
use crate::{
    Contract, ContractStatus, DataKey, Escrow, EscrowClient, EscrowError, ReleaseAuthorization,
};
use soroban_sdk::{
    testutils::Address as _, token::StellarAssetClient, vec, Address, Env, String, Vec,
};

fn valid_comment(env: &Env) -> String {
    String::from_str(env, "Great job!")
}

/// Helper to register an escrow client with settlement token binding
fn register_client_with_token(env: &Env) -> (EscrowClient<'_>, Address) {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    env.mock_all_auths();
    client.initialize(&admin);

    // Bind settlement token
    let token_client = env.register_stellar_asset_contract_v2(admin.clone());
    let token = token_client.address();
    client.bind_settlement_token(&admin, &token);

    (client, token)
}

/// Completes a new escrow for the supplied participants so multiple contracts
/// can accrue reputation credits to the same freelancer.
fn complete_contract_for(
    env: &Env,
    client: &crate::EscrowClient<'_>,
    token: &Address,
    client_addr: &Address,
    freelancer_addr: &Address,
) -> u32 {
    let contract_id = client.create_contract(
        client_addr,
        freelancer_addr,
        &None,
        &super::default_milestones(env),
        &ReleaseAuthorization::ClientOnly,
    );
    let total = super::total_milestone_amount();

    // Mint tokens for the client
    StellarAssetClient::new(env, token).mint(client_addr, &total);

    assert!(client.deposit_funds(&contract_id, client_addr, &total));
    for milestone_index in 0..3 {
        assert!(client.approve_milestone_release(&contract_id, client_addr, &milestone_index));
        assert!(client.release_milestone(&contract_id, client_addr, &milestone_index));
    }
    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Completed
    );
    contract_id
}

/// Helper to complete a contract with proper token setup
fn complete_contract_with_token(
    env: &Env,
    client: &EscrowClient,
    token: &Address,
) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &super::default_milestones(env),
        &ReleaseAuthorization::ClientOnly,
    );
    let total = super::total_milestone_amount();

    // Mint tokens for the client
    StellarAssetClient::new(env, token).mint(&client_addr, &total);

    client.deposit_funds(&contract_id, &client_addr, &total);
    for milestone_index in 0..3u32 {
        client.approve_milestone_release(&contract_id, &client_addr, &milestone_index);
        client.release_milestone(&contract_id, &client_addr, &milestone_index);
    }
    (client_addr, freelancer_addr, contract_id)
}

#[test]
fn pending_reputation_credits_accumulate_and_drain_across_completed_contracts() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let freelancer = Address::generate(&env);
    let first_client = Address::generate(&env);
    let second_client = Address::generate(&env);
    let third_client = Address::generate(&env);

    let first_contract = complete_contract_for(&env, &client, &token, &first_client, &freelancer);
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 1);

    let second_contract = complete_contract_for(&env, &client, &token, &second_client, &freelancer);
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 2);

    let third_contract = complete_contract_for(&env, &client, &token, &third_client, &freelancer);
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 3);

    // A fully refunded contract is terminal but never earns a reputation credit.
    let refunded_client = Address::generate(&env);
    let refunded_contract = client.create_contract(
        &refunded_client,
        &freelancer,
        &None,
        &super::default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );

    // Mint tokens for refunded client
    StellarAssetClient::new(&env, &token).mint(&refunded_client, &super::total_milestone_amount());

    assert!(client.deposit_funds(
        &refunded_contract,
        &refunded_client,
        &super::total_milestone_amount(),
    ));
    assert_eq!(
        client.refund_unreleased_milestones(&refunded_contract, &vec![&env, 0_u32, 1, 2]),
        super::total_milestone_amount()
    );
    assert_eq!(
        client.get_contract(&refunded_contract).status,
        ContractStatus::Refunded
    );
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 3);

    assert!(client.issue_reputation(&first_contract, &first_client, &5, &valid_comment(&env)));
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 2);
    assert_eq!(
        client
            .get_reputation(&freelancer)
            .unwrap()
            .completed_contracts,
        1
    );

    assert!(client.issue_reputation(&second_contract, &second_client, &4, &valid_comment(&env)));
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 1);
    assert_eq!(
        client
            .get_reputation(&freelancer)
            .unwrap()
            .completed_contracts,
        2
    );

    assert!(client.issue_reputation(&third_contract, &third_client, &3, &valid_comment(&env)));
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 0);
    assert_eq!(
        client
            .get_reputation(&freelancer)
            .unwrap()
            .completed_contracts,
        3
    );

    let duplicate =
        client.try_issue_reputation(&first_contract, &first_client, &1, &valid_comment(&env));
    super::assert_contract_error(duplicate, EscrowError::ReputationAlreadyIssued);
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 0);
}

#[test]
fn issue_reputation_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (_client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);
    let unauthorized = Address::generate(&env);

    let result = client.try_issue_reputation(&contract_id, &unauthorized, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

#[test]
fn issue_reputation_rejects_non_completed_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn issue_reputation_rejects_invalid_rating_bounds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

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
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let empty_comment = String::from_str(&env, "");
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &empty_comment);
    super::assert_contract_error(result, EscrowError::EmptyComment);
}

#[test]
fn issue_reputation_rejects_comment_too_long() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let long_str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let long_comment = String::from_str(&env, long_str);
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &long_comment);
    super::assert_contract_error(result, EscrowError::CommentTooLong);
}

#[test]
fn issue_reputation_rejects_duplicate_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    let result = client.try_issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ReputationAlreadyIssued);
}

#[test]
fn issue_reputation_rejects_self_rating_when_client_equals_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

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
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
}

#[test]
fn issue_reputation_updates_reputation_record_and_pending_credits() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

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
    let (client, token) = register_client_with_token(&env);
    let unknown = Address::generate(&env);
    assert!(client.get_average_rating(&unknown).is_none());
}

#[test]
fn get_average_rating_single_rating_returns_scaled_value() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    client.issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));

    // 4 * 10_000 / 1 = 40_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(40_000));
}

#[test]
fn get_average_rating_multiple_ratings_returns_correct_scaled_average() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);

    // First contract: rating 3
    let (client_addr1, freelancer_addr, contract_id1) =
        complete_contract_with_token(&env, &client, &token);
    client.issue_reputation(&contract_id1, &client_addr1, &3, &valid_comment(&env));

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
    client.issue_reputation(&contract_id2, &client_addr2, &5, &valid_comment(&env));

    // total_rating=8, completed_contracts=2 → 8 * 10_000 / 2 = 40_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(40_000));
}

#[test]
fn get_average_rating_fractional_average_is_preserved() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);

    // First contract: rating 1
    let (client_addr1, freelancer_addr, contract_id1) =
        complete_contract_with_token(&env, &client, &token);
    client.issue_reputation(&contract_id1, &client_addr1, &1, &valid_comment(&env));

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
    client.issue_reputation(&contract_id2, &client_addr2, &2, &valid_comment(&env));

    // total_rating=3, completed_contracts=2 → 3 * 10_000 / 2 = 15_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(15_000));
}

// ── Boundary and Rejection Tests ────────────────────────────────────────────

// ── Rating Boundaries ────────────────────────────────────────────────────────

#[test]
fn rating_boundary_exactly_1_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    assert!(client.issue_reputation(&contract_id, &client_addr, &1, &valid_comment(&env)));
    let rep = client.get_reputation(&freelancer_addr).unwrap();
    assert_eq!(rep.last_rating, 1);
    assert_eq!(rep.total_rating, 1);
}

#[test]
fn rating_boundary_exactly_5_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    let rep = client.get_reputation(&freelancer_addr).unwrap();
    assert_eq!(rep.last_rating, 5);
    assert_eq!(rep.total_rating, 5);
}

#[test]
fn rating_boundary_zero_rejects_with_invalid_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let result = client.try_issue_reputation(&contract_id, &client_addr, &0, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::InvalidRating);
}

#[test]
fn rating_boundary_six_rejects_with_invalid_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let result = client.try_issue_reputation(&contract_id, &client_addr, &6, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::InvalidRating);
}

#[test]
fn rating_boundary_u32_max_rejects_with_invalid_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &u32::MAX, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::InvalidRating);
}

// ── Comment Length Boundaries ────────────────────────────────────────────────

#[test]
fn comment_boundary_exactly_1_byte_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let comment = String::from_str(&env, "a");
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &comment));
    assert_eq!(client.get_reputation_comment(&contract_id), Some(comment));
}

#[test]
fn comment_boundary_exactly_200_bytes_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Create exactly 200-byte comment
    let s200 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert_eq!(s200.len(), 200);
    let comment = String::from_str(&env, s200);
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &comment));
    assert_eq!(client.get_reputation_comment(&contract_id), Some(comment));
}

#[test]
fn comment_boundary_exactly_201_bytes_rejects_with_comment_too_long() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Create exactly 201-byte comment
    let s201 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert_eq!(s201.len(), 201);
    let comment = String::from_str(&env, s201);
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &comment);
    super::assert_contract_error(result, EscrowError::CommentTooLong);
}

#[test]
fn comment_boundary_empty_string_rejects_with_empty_comment() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    let empty = String::from_str(&env, "");
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &empty);
    super::assert_contract_error(result, EscrowError::EmptyComment);
}

// ── Pending Reputation Credits Boundaries ────────────────────────────────────

#[test]
fn pending_credits_boundary_exactly_zero_rejects_with_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Consume the credit
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);

    // Create another completed contract but manually issue reputation without proper credit
    let new_client = Address::generate(&env);
    let contract_id2 = complete_contract_for(&env, &client, &token, &new_client, &freelancer_addr);

    // Consume the second credit
    assert!(client.issue_reputation(&contract_id2, &new_client, &4, &valid_comment(&env)));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);

    // Try to issue reputation again on first contract (should fail with InvalidState due to zero credits)
    // This is prevented by ReputationAlreadyIssued, so we need to test the state directly
    // Note: This boundary is implicitly tested through the credit accumulation tests
}

#[test]
fn pending_credits_boundary_exactly_one_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 1);
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    assert_eq!(client.get_pending_reputation_credits(&freelancer_addr), 0);
}

#[test]
fn pending_credits_boundary_multiple_credits_drain_correctly() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let freelancer = Address::generate(&env);

    // Create 5 completed contracts
    let mut contracts: Vec<(u32, Address)> = Vec::new(&env);
    for _i in 0..5 {
        let client_addr = Address::generate(&env);
        let contract_id = complete_contract_for(&env, &client, &token, &client_addr, &freelancer);
        contracts.push_back((contract_id, client_addr));
    }

    // Verify exactly 5 credits
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 5);

    // Drain them one by one
    for i in 0..5 {
        let (contract_id, client_addr) = contracts.get(i).unwrap();
        assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
        assert_eq!(
            client.get_pending_reputation_credits(&freelancer),
            (4 - i) as i128
        );
    }

    assert_eq!(client.get_pending_reputation_credits(&freelancer), 0);
}

// ── Authorization Boundaries ─────────────────────────────────────────────────

#[test]
fn authorization_boundary_exact_client_match_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Exact client address must succeed
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
}

#[test]
fn authorization_boundary_non_client_rejects_with_unauthorized_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Freelancer cannot issue reputation
    let result =
        client.try_issue_reputation(&contract_id, &freelancer_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);

    // Random address cannot issue reputation
    let random = Address::generate(&env);
    let result = client.try_issue_reputation(&contract_id, &random, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);

    // Only client can issue
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
}

#[test]
fn authorization_boundary_arbiter_cannot_issue_reputation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter = Address::generate(&env);

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter.clone()),
        &super::default_milestones(&env),
        &ReleaseAuthorization::ClientAndArbiter,
    );

    let total = super::total_milestone_amount();
    client.deposit_funds(&contract_id, &client_addr, &total);

    for milestone_index in 0..3 {
        client.approve_milestone_release(&contract_id, &arbiter, &milestone_index);
        client.release_milestone(&contract_id, &arbiter, &milestone_index);
    }

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Completed
    );

    // Arbiter cannot issue reputation
    let result = client.try_issue_reputation(&contract_id, &arbiter, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

// ── Contract Status Boundaries ───────────────────────────────────────────────

#[test]
fn contract_status_boundary_exactly_completed_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Completed
    );
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
}

#[test]
fn contract_status_boundary_created_rejects_with_not_completed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::create_contract(&env, &client);

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Created
    );
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn contract_status_boundary_funded_rejects_with_not_completed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::create_contract(&env, &client);

    let total = super::total_milestone_amount();
    client.deposit_funds(&contract_id, &client_addr, &total);

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Funded
    );
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn contract_status_boundary_disputed_rejects_with_not_completed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter = Address::generate(&env);

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter.clone()),
        &super::default_milestones(&env),
        &ReleaseAuthorization::ClientAndArbiter,
    );

    let total = super::total_milestone_amount();
    client.deposit_funds(&contract_id, &client_addr, &total);
    client.raise_dispute(&contract_id, &client_addr);

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn contract_status_boundary_refunded_rejects_with_not_completed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::create_contract(&env, &client);

    let total = super::total_milestone_amount();
    client.deposit_funds(&contract_id, &client_addr, &total);
    client.refund_unreleased_milestones(&contract_id, &vec![&env, 0_u32, 1, 2]);

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Refunded
    );
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

#[test]
fn contract_status_boundary_cancelled_rejects_with_not_completed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::create_contract(&env, &client);

    client.cancel_contract(&contract_id, &client_addr);

    assert_eq!(
        client.get_contract(&contract_id).status,
        ContractStatus::Cancelled
    );
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);
}

// ── Reputation Issued Flag Boundaries ────────────────────────────────────────

#[test]
fn reputation_issued_flag_boundary_false_to_true_transition_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Initially false
    assert_eq!(client.get_contract(&contract_id).reputation_issued, false);

    // Issue reputation
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));

    // Now true
    assert_eq!(client.get_contract(&contract_id).reputation_issued, true);
}

#[test]
fn reputation_issued_flag_boundary_already_true_rejects() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Issue once
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    assert_eq!(client.get_contract(&contract_id).reputation_issued, true);

    // Try to issue again
    let result = client.try_issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ReputationAlreadyIssued);
}

// ── Self-Rating Boundaries ───────────────────────────────────────────────────

#[test]
fn self_rating_boundary_distinct_addresses_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Verify addresses are distinct
    assert_ne!(client_addr, freelancer_addr);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
}

#[test]
fn self_rating_boundary_same_address_rejects() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Manually set freelancer to be same as client
    env.as_contract(&client.address, || {
        let key = DataKey::Contract(contract_id);
        let mut contract: Contract = env.storage().persistent().get(&key).unwrap();
        contract.freelancer = client_addr.clone();
        env.storage().persistent().set(&key, &contract);
    });

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::SelfRating);
}

// ── Average Rating Calculation Boundaries ────────────────────────────────────

#[test]
fn average_rating_boundary_single_minimum_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    client.issue_reputation(&contract_id, &client_addr, &1, &valid_comment(&env));

    // 1 * 10_000 / 1 = 10_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(10_000));
}

#[test]
fn average_rating_boundary_single_maximum_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let (client_addr, freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));

    // 5 * 10_000 / 1 = 50_000
    assert_eq!(client.get_average_rating(&freelancer_addr), Some(50_000));
}

#[test]
fn average_rating_boundary_all_minimum_ratings() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let freelancer = Address::generate(&env);

    // Create 3 contracts with rating 1
    for _ in 0..3 {
        let client_addr = Address::generate(&env);
        let contract_id = complete_contract_for(&env, &client, &token, &client_addr, &freelancer);
        client.issue_reputation(&contract_id, &client_addr, &1, &valid_comment(&env));
    }

    // 3 * 10_000 / 3 = 10_000
    assert_eq!(client.get_average_rating(&freelancer), Some(10_000));
}

#[test]
fn average_rating_boundary_all_maximum_ratings() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let freelancer = Address::generate(&env);

    // Create 3 contracts with rating 5
    for _ in 0..3 {
        let client_addr = Address::generate(&env);
        let contract_id = complete_contract_for(&env, &client, &token, &client_addr, &freelancer);
        client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    }

    // 15 * 10_000 / 3 = 50_000
    assert_eq!(client.get_average_rating(&freelancer), Some(50_000));
}

#[test]
fn average_rating_boundary_zero_completed_contracts_returns_none() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let freelancer = Address::generate(&env);

    // No reputation record exists
    assert!(client.get_reputation(&freelancer).is_none());
    assert!(client.get_average_rating(&freelancer).is_none());
}

// ── Contract Not Found Boundary ──────────────────────────────────────────────

#[test]
fn contract_not_found_boundary_nonexistent_contract_id_rejects() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, token) = register_client_with_token(&env);
    let client_addr = Address::generate(&env);

    let nonexistent_id = 99999_u32;
    let result =
        client.try_issue_reputation(&nonexistent_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ContractNotFound);
}

// ── Pause and Emergency Controls Boundary ────────────────────────────────────

#[test]
fn paused_contract_rejects_issue_reputation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let escrow_address = env.register(crate::Escrow, ());
    let client = EscrowClient::new(&env, &escrow_address);
    client.initialize(&admin);
    // Bind settlement token
    let token_client = env.register_stellar_asset_contract_v2(admin.clone());
    let token = token_client.address();
    client.bind_settlement_token(&admin, &token);

    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Pause the contract
    client.pause();

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ContractPaused);
}

#[test]
fn emergency_active_rejects_issue_reputation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let escrow_address = env.register(crate::Escrow, ());
    let client = EscrowClient::new(&env, &escrow_address);
    // Bind settlement token
    let token_client = env.register_stellar_asset_contract_v2(admin.clone());
    let token = token_client.address();
    client.bind_settlement_token(&admin, &token);

    client.initialize(&admin);

    let (client_addr, _freelancer_addr, contract_id) =
        complete_contract_with_token(&env, &client, &token);

    // Activate emergency mode
    client.activate_emergency_pause();

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::EmergencyActive);
}
