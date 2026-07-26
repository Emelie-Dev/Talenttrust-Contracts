use super::{complete_contract, create_contract, register_client};
use crate::{Contract, ContractStatus, DataKey, EscrowError, ReleaseAuthorization};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events},
    vec, Address, Env, FromVal, String, Symbol, TryFromVal, Val, Vec,
};
fn valid_comment(env: &Env) -> String {
    String::from_str(env, "Great job!")
}

// ---------------------------------------------------------------------------
// Helpers for asserting on `rep_issue` events (issue #944)
//
// These helpers are std-free so they keep `#![cfg(test)]` consistent with
// the rest of the contract's test suite (which uses `soroban_sdk::Vec`
// exclusively) and avoid pulling `std::collections` into a `#![no_std]`
// crate's test build.
// ---------------------------------------------------------------------------

/// Extract the first topic of `ev` as a `Symbol`, if present.
fn first_topic(env: &Env, ev: &(Address, Vec<Val>, Val)) -> Option<Symbol> {
    if ev.1.len() == 0 {
        return None;
    }
    Symbol::try_from_val(env, &ev.1.get(0).unwrap()).ok()
}

/// True if the first topic of `ev` equals `want`.
fn has_topic(env: &Env, ev: &(Address, Vec<Val>, Val), want: Symbol) -> bool {
    first_topic(env, ev).map(|s| s == want).unwrap_or(false)
}

/// Total number of events in the host whose first topic equals `want`.
fn count_topic(env: &Env, want: Symbol) -> u32 {
    env.events()
        .all()
        .iter()
        .filter(|ev| has_topic(env, ev, want.clone()))
        .count() as u32
}

/// Decode the data payload of a `rep_issue` event into the published
/// tuple shape: `(client, freelancer, rating, total_rating, completed_contracts, timestamp)`.
type RepIssuePayload = (Address, Address, u32, i128, i128, u64);
fn decode_rep_issue_payload(env: &Env, payload: &Val) -> RepIssuePayload {
    <RepIssuePayload>::from_val(env, payload)
}

/// Completes a new escrow for the supplied participants so multiple contracts
/// can accrue reputation credits to the same freelancer.
fn complete_contract_for(
    env: &Env,
    client: &crate::EscrowClient<'_>,
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

#[test]
fn pending_reputation_credits_accumulate_and_drain_across_completed_contracts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let freelancer = Address::generate(&env);
    let first_client = Address::generate(&env);
    let second_client = Address::generate(&env);
    let third_client = Address::generate(&env);

    let first_contract = complete_contract_for(&env, &client, &first_client, &freelancer);
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 1);

    let second_contract = complete_contract_for(&env, &client, &second_client, &freelancer);
    assert_eq!(client.get_pending_reputation_credits(&freelancer), 2);

    let third_contract = complete_contract_for(&env, &client, &third_client, &freelancer);
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

    client.issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));

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
    let client = register_client(&env);

    // First contract: rating 1
    let (client_addr1, freelancer_addr, contract_id1) = complete_contract(&env, &client);
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

#[test]
fn issue_reputation_rejects_invalid_contract_id_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);

    let result = client.try_issue_reputation(&0, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::InvalidContractId);
}

/// Happy-path: `issue_reputation` publishes exactly one `rep_issue` event
/// whose topics are `(symbol_short!("rep_issue"), contract_id)` and whose
/// payload carries every id/amount required for off-chain reconstruction.
#[test]
fn issue_reputation_emits_rep_issue_event_with_correct_topic_and_payload() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = complete_contract(&env, &client);

    let rating: u32 = 4;
    assert!(client.issue_reputation(&contract_id, &client_addr, &rating, &valid_comment(&env)));

    // Locate the (single) rep_issue event for this contract.
    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        1,
        "expected exactly one rep_issue event"
    );
    let (publisher, topics, payload) = find_rep_issue_event(&env, contract_id);

    // Publisher must be the escrow contract address.
    assert_eq!(publisher, client.address);

    // Topics: (symbol_short!("rep_issue"), contract_id)
    assert_eq!(topics.len(), 2, "expected exactly 2 topics");
    let topic0 = first_topic(&env, &(publisher, topics.clone(), payload.clone()))
        .expect("first topic missing");
    assert_eq!(topic0, symbol_short!("rep_issue"));
    let topic1: u32 = u32::from_val(&env, &topics.get(1).unwrap());
    assert_eq!(topic1, contract_id);

    // Payload: (client, freelancer, rating, total_rating, completed_contracts, timestamp)
    let expected_payload: RepIssuePayload = (
        client_addr.clone(),
        freelancer_addr.clone(),
        rating,
        rating as i128, // first issuance -> total_rating == rating
        1i128,          // first issuance -> completed_contracts == 1
        env.ledger().timestamp(),
    );
    let actual_payload = decode_rep_issue_payload(&env, &payload);
    assert_eq!(actual_payload, expected_payload);
}

/// Interop test: the publisher's `symbol_short!("rep_issue")` literal must
/// be value-equal to `Symbol::new(&env, "rep_issue")` (i.e. the runtime
/// event-decoder path produces the same symbol a downstream indexer would
/// construct from the string topic name). This guards against a regression
/// where a developer accidentally uses a non-`symbol_short!` symbol.
#[test]
fn issue_reputation_topic_matches_runtime_symbol_new_string() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer, contract_id) = complete_contract(&env, &client);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));

    assert_eq!(count_topic(&env, symbol_short!("rep_issue")), 1);
    let (_pub, topics, _payload) = find_rep_issue_event(&env, contract_id);

    let topic_short = symbol_short!("rep_issue");
    let topic_long: Symbol = Symbol::new(&env, "rep_issue");
    let decoded = first_topic(&env, &(client.address.clone(), topics, _payload))
        .expect("first topic missing");
    assert_eq!(
        decoded, topic_short,
        "symbol_short!(\"rep_issue\") must publish exactly that symbol"
    );
    assert_eq!(
        decoded, topic_long,
        "publisher symbol must equal Symbol::new(\"rep_issue\") for cross-tooling interop"
    );
}

/// Each successful `issue_reputation` produces exactly one event, scoped to
/// the issuing contract_id. Indexers rely on this for a clean per-contract
/// ledger.
#[test]
fn issue_reputation_emits_one_event_per_successful_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let freelancer = Address::generate(&env);
    let first_client = Address::generate(&env);
    let second_client = Address::generate(&env);
    let first_contract = complete_contract_for(&env, &client, &first_client, &freelancer);
    let second_contract = complete_contract_for(&env, &client, &second_client, &freelancer);

    assert!(client.issue_reputation(&first_contract, &first_client, &5, &valid_comment(&env)));
    assert!(client.issue_reputation(&second_contract, &second_client, &3, &valid_comment(&env)));

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        2,
        "expected one rep_issue event per contract"
    );
}

/// Cumulative totals in the payload must track each new issuance correctly.
/// This lets indexers compute running averages without re-fetching storage.
#[test]
fn issue_reputation_event_payload_reflects_running_totals() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let freelancer = Address::generate(&env);
    let a_client = Address::generate(&env);
    let b_client = Address::generate(&env);
    let a_contract = complete_contract_for(&env, &client, &a_client, &freelancer);
    let b_contract = complete_contract_for(&env, &client, &b_client, &freelancer);

    // First issuance: rating 4 -> totals (4, 1)
    client.issue_reputation(&a_contract, &a_client, &4, &valid_comment(&env));
    // Second issuance: rating 2 -> totals (6, 2)
    client.issue_reputation(&b_contract, &b_client, &2, &valid_comment(&env));

    assert_eq!(count_topic(&env, symbol_short!("rep_issue")), 2);
    let (_, _, a_payload_val) = find_rep_issue_event(&env, a_contract);
    let (_, _, b_payload_val) = find_rep_issue_event(&env, b_contract);
    let a_payload = decode_rep_issue_payload(&env, &a_payload_val);
    let b_payload = decode_rep_issue_payload(&env, &b_payload_val);

    assert_eq!(a_payload.2, 4); // rating
    assert_eq!(a_payload.3, 4); // total_rating after first
    assert_eq!(a_payload.4, 1); // completed_contracts after first
    assert_eq!(a_payload.0, a_client);
    assert_eq!(a_payload.1, freelancer);

    assert_eq!(b_payload.2, 2); // rating
    assert_eq!(b_payload.3, 6); // total_rating after second (4+2)
    assert_eq!(b_payload.4, 2); // completed_contracts after second
    assert_eq!(b_payload.0, b_client);
    assert_eq!(b_payload.1, freelancer);
}

/// No-collision: walking every emitted event in this test, no two events
/// share the same first topic. A regression that re-introduces a duplicate
/// symbol_short! literal anywhere in the contract would surface here as
/// long as that path was exercised in the same test scope. We also
/// cross-check `rep_issue` against a known set of other short-symbol
/// topics to guarantee the string-valued `Symbol::new` form doesn't
/// silently collide with a future long-form rename.
#[test]
fn issue_reputation_event_topic_does_not_collide_with_other_topics() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    // Drive a `rep_issue` emission.
    let (client_addr, _freelancer, contract_id) = complete_contract(&env, &client);
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));

    // Collect the first topic of every emitted event into a `Vec<Symbol>`
    // and assert uniqueness via linear scan (no std::collections::HashSet
    // in #![cfg(test)]).
    let mut seen_topics: Vec<Symbol> = Vec::new(&env);
    for ev in env.events().all().iter() {
        if let Some(sym) = first_topic(&env, &ev) {
            for prior in seen_topics.iter() {
                assert_ne!(
                    sym, prior,
                    "duplicate first topic emitted (event-topic collision!)"
                );
            }
            seen_topics.push_back(sym);
        }
    }
    // Sanity: we collected at least one topic (the rep_issue we just emitted).
    assert!(seen_topics.len() >= 1);

    // Cross-check well-known short-symbol topic literals. None of these
    // may collide with `rep_issue`. (If a future change adds one of these
    // strings as a topic, this assertion guards the indexing surface.)
    let sibling_short_topics: &[&str] = &[
        "init",
        "mlstn_rls",
        "ctrct_cmp",
        "refunded",
        "pause",
        "unpaused",
        "cancelled",
        "evidence",
        "fee",
        "dispute",
    ];
    for name in sibling_short_topics.iter() {
        assert_ne!(
            *name, "rep_issue",
            "topic collision with existing short-symbol name: {}",
            name
        );
        assert!(
            name.len() <= 9,
            "sibling short topic {} exceeds symbol_short! 9-char limit",
            name
        );
    }
}

/// Fail-closed: NO `rep_issue` event is published when the caller is
/// unauthorized.
#[test]
fn issue_reputation_does_not_emit_event_on_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);
    let unauthorized = Address::generate(&env);

    let result = client.try_issue_reputation(&contract_id, &unauthorized, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted when the caller is unauthorized (fail-closed)"
    );
}

/// Fail-closed: invalid rating bound must not publish a `rep_issue` event.
#[test]
fn issue_reputation_does_not_emit_event_on_invalid_rating() {
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

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted when the rating is out of bounds"
    );
}

/// Fail-closed: empty / oversized comment must not publish a `rep_issue`
/// event.
#[test]
fn issue_reputation_does_not_emit_event_on_invalid_comment() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    let empty = String::from_str(&env, "");
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &empty);
    super::assert_contract_error(result, EscrowError::EmptyComment);

    let long_str = "x".repeat(250); // > 200 byte cap
    let long_comment = String::from_str(&env, &long_str);
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &long_comment);
    super::assert_contract_error(result, EscrowError::CommentTooLong);

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted on invalid comment"
    );
}

/// Fail-closed: a successful issuance publishes exactly one `rep_issue`
/// event; a duplicate attempt is rejected and does NOT publish a second.
#[test]
fn issue_reputation_does_not_emit_event_on_duplicate_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env)));
    let result = client.try_issue_reputation(&contract_id, &client_addr, &4, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ReputationAlreadyIssued);

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        1,
        "duplicate issuance must not produce a second rep_issue event"
    );
}

/// Fail-closed: reputation issued against a non-Completed contract must not
/// publish a `rep_issue` event.
#[test]
fn issue_reputation_does_not_emit_event_on_unfinished_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::NotCompleted);

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted when the contract has not been completed"
    );
}/// Self-rating must not publish a `rep_issue` event.
#[test]
fn issue_reputation_does_not_emit_event_on_self_rating() {
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

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted when the client is the freelancer"
    );
}

/// Contract-paused: `issue_reputation` short-circuits via `require_not_paused`
/// BEFORE any state mutation, so a paused contract must not publish a
/// `rep_issue` event.
#[test]
fn issue_reputation_does_not_emit_event_when_contract_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    // Pause the contract. Pause requires admin auth, which `mock_all_auths` covers.
    assert!(client.pause());

    // Reputations issued while paused must panic with ContractPaused.
    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::ContractPaused);

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted while the contract is paused (fail-closed)"
    );
}

/// Emergency-active: `activate_emergency_pause` sets both `Emergency` and
/// `Paused`. The `require_not_paused` guard in `issue_reputation` fires and
/// no `rep_issue` event is published.
#[test]
fn issue_reputation_does_not_emit_event_when_emergency_active() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = complete_contract(&env, &client);

    assert!(client.activate_emergency_pause());

    let result =
        client.try_issue_reputation(&contract_id, &client_addr, &5, &valid_comment(&env));
    super::assert_contract_error(result, EscrowError::EmergencyActive);

    assert_eq!(
        count_topic(&env, symbol_short!("rep_issue")),
        0,
        "rep_issue must NOT be emitted while emergency controls are active (fail-closed)"
    );
}
