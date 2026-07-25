//! Boundary and rejection tests for the escrow authorization logic.
//!
//! Issue #821 — Add boundary tests for the authorization logic.
//!
//! Complements the broader `release_authorization` and
//! `authorization_matrix_validation` suites by focusing exclusively on the
//! accept/reject *boundaries* of every authorization mode: the exact threshold
//! at which a call transitions from permitted to rejected, and vice-versa.
//!
//! # Coverage map
//!
//! | Boundary | Variants tested |
//! |---|---|
//! | Milestone index: 0 (first), last valid, one-past-end | approve + release |
//! | Approval count: 0, exactly required, one short | all four modes |
//! | Approver identity: each participant + stranger | all four modes |
//! | Release caller: each participant + stranger | all four modes |
//! | Duplicate approval (second call same party) | ClientOnly, MultiSig |
//! | Missing arbiter at creation | ArbiterOnly, ClientAndArbiter |
//! | Wrong-party approval does not unlock release | ArbiterOnly, ClientOnly |
//! | Paused contract blocks approve + release | representative mode |
//! | InvalidState: Created, Completed, Cancelled statuses | approve + release |
//! | Approval TTL expiry | ClientOnly |

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, vec, Address, Env};

use crate::{
    ContractStatus, DataKey, Error, Escrow, EscrowClient, EscrowError, ReleaseAuthorization,
};

use super::assert_contract_error;

// ---------------------------------------------------------------------------
// Test-local helpers
// ---------------------------------------------------------------------------

/// Two milestones: 500 + 300 = 800 total (no stroops for simplicity).
fn milestones(env: &Env) -> soroban_sdk::Vec<i128> {
    vec![env, 500_i128, 300_i128]
}

fn total() -> i128 {
    800_i128
}

/// Register a fresh escrow contract and return an initialized client.
fn new_client(env: &Env) -> EscrowClient<'_> {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    client
}

/// Register a fresh escrow contract and return both the client and admin address.
fn new_client_with_admin(env: &Env) -> (EscrowClient<'_>, Address) {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (client, admin)
}

/// Generate the standard three test participants.
fn participants(env: &Env) -> (Address, Address, Address) {
    (
        Address::generate(env),
        Address::generate(env),
        Address::generate(env),
    )
}

/// Create a contract (not yet funded) and return its id.
fn create(
    env: &Env,
    client: &EscrowClient<'_>,
    client_addr: &Address,
    freelancer_addr: &Address,
    arbiter: Option<&Address>,
    auth: &ReleaseAuthorization,
) -> u32 {
    client.create_contract(
        client_addr,
        freelancer_addr,
        &arbiter.cloned(),
        &milestones(env),
        auth,
    )
}

/// Inject `Funded` status and `funded_amount` directly into persistent storage
/// so approval and release checks can run without a bound SAC token.
fn inject_funded(env: &Env, escrow_addr: &Address, contract_id: u32) {
    env.as_contract(escrow_addr, || {
        let key = DataKey::Contract(contract_id);
        let mut c: crate::Contract = env.storage().persistent().get(&key).unwrap();
        c.status = ContractStatus::Funded;
        c.funded_amount = total();
        env.storage().persistent().set(&key, &c);
    });
}

/// Create a contract and immediately inject `Funded` status via storage.
/// No SAC token is required.
fn create_funded(
    env: &Env,
    client: &EscrowClient<'_>,
    client_addr: &Address,
    freelancer_addr: &Address,
    arbiter: Option<&Address>,
    auth: &ReleaseAuthorization,
) -> u32 {
    let id = create(env, client, client_addr, freelancer_addr, arbiter, auth);
    inject_funded(env, &client.address, id);
    id
}

/// Inject an arbitrary `ContractStatus` via storage (for InvalidState tests).
fn inject_status(env: &Env, escrow_addr: &Address, contract_id: u32, status: ContractStatus) {
    env.as_contract(escrow_addr, || {
        let key = DataKey::Contract(contract_id);
        let mut c: crate::Contract = env.storage().persistent().get(&key).unwrap();
        c.status = status;
        env.storage().persistent().set(&key, &c);
    });
}

// ===========================================================================
// Section 1 — Milestone index boundaries
//
// The approve and release entry-points must accept index 0 (first milestone)
// and the last valid index (len - 1), and must reject index == len
// (one-past-the-end) with IndexOutOfBounds.
// ===========================================================================

/// Approval at milestone index 0 (the lowest valid index) is accepted.
#[test]
fn approve_at_index_zero_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
}

/// Approval at the last valid milestone index (len - 1) is accepted.
#[test]
fn approve_at_last_valid_index_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    // milestones() has 2 entries; last valid index = 1
    assert!(client.approve_milestone_release(&id, &client_addr, &1));
}

/// Approval at one-past-the-end (index == len) is rejected with IndexOutOfBounds.
#[test]
fn approve_at_out_of_bounds_index_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    // len = 2, so index 2 is one-past-the-end.
    // approvals::approve_milestone returns Error::IndexOutOfBounds which is
    // panicked via env.panic_with_error — numeric code 3.
    let result = client.try_approve_milestone_release(&id, &client_addr, &2);
    assert_contract_error(result, Error::IndexOutOfBounds);
}

/// Release at one-past-the-end (index == len) is rejected.
#[test]
fn release_at_out_of_bounds_index_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    client.approve_milestone_release(&id, &client_addr, &0);

    // index 2 is beyond the two-milestone contract
    let result = client.try_release_milestone(&id, &client_addr, &2);
    assert!(result.is_err());
}

// ===========================================================================
// Section 2 — Approval count boundaries (zero, one-short, exactly required)
//
// Each mode has a minimum required approval count. These tests verify:
//   - zero approvals  → InsufficientApprovals
//   - one short       → InsufficientApprovals  (MultiSig only)
//   - exactly enough  → release succeeds
// ===========================================================================

// --- ClientOnly ---

/// ClientOnly, zero approvals: release rejected with InsufficientApprovals.
#[test]
fn client_only_zero_approvals_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

/// ClientOnly, exactly one approval (client): release succeeds.
#[test]
fn client_only_exactly_required_approval_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    // Exactly the required approval is present — release must succeed.
    assert!(client.release_milestone(&id, &client_addr, &0));
}

// --- ArbiterOnly ---

/// ArbiterOnly, zero approvals: release rejected with InsufficientApprovals.
#[test]
fn arbiter_only_zero_approvals_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );

    let result = client.try_release_milestone(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

/// ArbiterOnly, exactly arbiter approval: release succeeds.
#[test]
fn arbiter_only_exactly_required_approval_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );

    assert!(client.approve_milestone_release(&id, &arbiter_addr, &0));
    assert!(client.release_milestone(&id, &arbiter_addr, &0));
}

// --- ClientAndArbiter ---

/// ClientAndArbiter, zero approvals: release rejected.
#[test]
fn client_and_arbiter_zero_approvals_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

/// ClientAndArbiter, only client approval (OR logic): release succeeds.
#[test]
fn client_and_arbiter_client_alone_is_sufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    assert!(client.release_milestone(&id, &client_addr, &0));
}

/// ClientAndArbiter, only arbiter approval (OR logic): release succeeds.
#[test]
fn client_and_arbiter_arbiter_alone_is_sufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );

    assert!(client.approve_milestone_release(&id, &arbiter_addr, &0));
    assert!(client.release_milestone(&id, &arbiter_addr, &0));
}

// --- MultiSig ---

/// MultiSig, zero approvals: release rejected.
#[test]
fn multisig_zero_approvals_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

/// MultiSig, only one approval (client, not freelancer): one short → rejected.
#[test]
fn multisig_one_short_client_only_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

/// MultiSig, only one approval (freelancer, not client): one short → rejected.
#[test]
fn multisig_one_short_freelancer_only_insufficient() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    assert!(client.approve_milestone_release(&id, &freelancer_addr, &0));
    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::InsufficientApprovals);
}

/// MultiSig, both approvals present (exactly required): release succeeds.
#[test]
fn multisig_both_approvals_exactly_required_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    assert!(client.approve_milestone_release(&id, &freelancer_addr, &0));
    assert!(client.release_milestone(&id, &client_addr, &0));
}

// ===========================================================================
// Section 3 — Duplicate approval boundaries
//
// A party may approve exactly once per milestone. The second call from the
// same party must be rejected with AlreadyApproved regardless of mode.
// ===========================================================================

/// ClientOnly: client approving twice → second call rejected with AlreadyApproved.
#[test]
fn client_only_duplicate_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, Error::AlreadyApproved);
}

/// MultiSig: client approving twice → second call rejected with AlreadyApproved.
#[test]
fn multisig_client_duplicate_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, Error::AlreadyApproved);
}

/// MultiSig: freelancer approving twice → second call rejected with AlreadyApproved.
#[test]
fn multisig_freelancer_duplicate_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    assert!(client.approve_milestone_release(&id, &freelancer_addr, &0));
    let result = client.try_approve_milestone_release(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::AlreadyApproved);
}

/// ArbiterOnly: arbiter approving twice → second call rejected with AlreadyApproved.
#[test]
fn arbiter_only_duplicate_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );

    assert!(client.approve_milestone_release(&id, &arbiter_addr, &0));
    let result = client.try_approve_milestone_release(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::AlreadyApproved);
}

// ===========================================================================
// Section 4 — Approver identity boundaries
//
// Each mode restricts who may call `approve_milestone_release`. Attempts by
// any party not listed as an allowed approver must return UnauthorizedRole.
// ===========================================================================

/// ClientOnly: freelancer attempting to approve → UnauthorizedRole.
#[test]
fn client_only_freelancer_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_approve_milestone_release(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// ClientOnly: arbiter attempting to approve → UnauthorizedRole.
#[test]
fn client_only_arbiter_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_approve_milestone_release(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// ClientOnly: stranger attempting to approve → UnauthorizedRole.
#[test]
fn client_only_stranger_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);
    let stranger = Address::generate(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_approve_milestone_release(&id, &stranger, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// ArbiterOnly: client attempting to approve → UnauthorizedRole.
#[test]
fn arbiter_only_client_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );

    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// ArbiterOnly: freelancer attempting to approve → UnauthorizedRole.
#[test]
fn arbiter_only_freelancer_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );

    let result = client.try_approve_milestone_release(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// ClientAndArbiter: freelancer attempting to approve → UnauthorizedRole.
#[test]
fn client_and_arbiter_freelancer_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );

    let result = client.try_approve_milestone_release(&id, &freelancer_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// MultiSig: arbiter attempting to approve → UnauthorizedRole.
#[test]
fn multisig_arbiter_cannot_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::MultiSig,
    );

    let result = client.try_approve_milestone_release(&id, &arbiter_addr, &0);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// Any mode: stranger (non-participant) attempting to approve → UnauthorizedRole.
#[test]
fn stranger_cannot_approve_any_mode() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);
    let stranger = Address::generate(&env);

    // ClientOnly
    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_contract_error(
        client.try_approve_milestone_release(&id, &stranger, &0),
        Error::UnauthorizedRole,
    );

    // ArbiterOnly
    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    assert_contract_error(
        client.try_approve_milestone_release(&id, &stranger, &0),
        Error::UnauthorizedRole,
    );

    // ClientAndArbiter
    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    assert_contract_error(
        client.try_approve_milestone_release(&id, &stranger, &0),
        Error::UnauthorizedRole,
    );

    // MultiSig
    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );
    assert_contract_error(
        client.try_approve_milestone_release(&id, &stranger, &0),
        Error::UnauthorizedRole,
    );
}

// ===========================================================================
// Section 5 — Release caller boundaries
//
// Each mode restricts who may call `release_milestone`. These tests confirm
// the exact boundary: each unauthorized caller is rejected with UnauthorizedRole
// even when valid approvals are present.
// ===========================================================================

/// ClientOnly: freelancer cannot release even when client approval exists.
#[test]
fn client_only_freelancer_cannot_release_with_valid_approval() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    client.approve_milestone_release(&id, &client_addr, &0);

    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, EscrowError::UnauthorizedRole);
}

/// ArbiterOnly: client cannot release even when arbiter approval exists.
#[test]
fn arbiter_only_client_cannot_release_with_valid_approval() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    client.approve_milestone_release(&id, &arbiter_addr, &0);

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, EscrowError::UnauthorizedRole);
}

/// ArbiterOnly: freelancer cannot release even when arbiter approval exists.
#[test]
fn arbiter_only_freelancer_cannot_release_with_valid_approval() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );
    client.approve_milestone_release(&id, &arbiter_addr, &0);

    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, EscrowError::UnauthorizedRole);
}

/// ClientAndArbiter: freelancer cannot release even when client approval exists.
#[test]
fn client_and_arbiter_freelancer_cannot_release_with_valid_approval() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    client.approve_milestone_release(&id, &client_addr, &0);

    let result = client.try_release_milestone(&id, &freelancer_addr, &0);
    assert_contract_error(result, EscrowError::UnauthorizedRole);
}

/// MultiSig: arbiter cannot release even when both client and freelancer approved.
#[test]
fn multisig_arbiter_cannot_release_with_both_approvals() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::MultiSig,
    );
    client.approve_milestone_release(&id, &client_addr, &0);
    client.approve_milestone_release(&id, &freelancer_addr, &0);

    let result = client.try_release_milestone(&id, &arbiter_addr, &0);
    assert_contract_error(result, EscrowError::UnauthorizedRole);
}

// ===========================================================================
// Section 6 — Missing-arbiter boundary at contract creation
//
// ArbiterOnly and ClientAndArbiter modes require an arbiter at creation time.
// Attempting to create either mode with no arbiter must fail. MultiSig and
// ClientOnly must succeed without an arbiter.
// ===========================================================================

/// ArbiterOnly without arbiter → creation rejected.
#[test]
fn arbiter_only_requires_arbiter_at_creation() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let result = client.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::ArbiterOnly,
    );
    assert!(
        result.is_err(),
        "ArbiterOnly requires an arbiter at creation"
    );
}

/// ClientAndArbiter without arbiter → creation rejected.
#[test]
fn client_and_arbiter_requires_arbiter_at_creation() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let result = client.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::ClientAndArbiter,
    );
    assert!(
        result.is_err(),
        "ClientAndArbiter requires an arbiter at creation"
    );
}

/// ClientOnly without arbiter → creation succeeds.
#[test]
fn client_only_does_not_require_arbiter() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let result = client.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(result.is_ok(), "ClientOnly must not require an arbiter");
}

/// MultiSig without arbiter → creation succeeds.
#[test]
fn multisig_does_not_require_arbiter() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let result = client.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones(&env),
        &ReleaseAuthorization::MultiSig,
    );
    assert!(result.is_ok(), "MultiSig must not require an arbiter");
}

// ===========================================================================
// Section 7 — InvalidState boundaries
//
// Both `approve_milestone_release` and `release_milestone` require the
// contract to be in `Funded` status. Any other status (Created, Completed,
// Cancelled) must produce `InvalidState` before role or approval checks run.
// ===========================================================================

/// Approve on a Created (unfunded) contract → InvalidState.
#[test]
fn approve_on_created_contract_fails_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    // create_contract leaves status = Created (no deposit).
    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, Error::InvalidState);
}

/// Release on a Created contract → InvalidState fires before role check.
#[test]
fn release_on_created_contract_fails_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InvalidState);
}

/// Release on a Completed contract → InvalidState.
#[test]
fn release_on_completed_contract_fails_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    inject_status(&env, &client.address, id, ContractStatus::Completed);

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InvalidState);
}

/// Approve on a Completed contract → InvalidState.
#[test]
fn approve_on_completed_contract_fails_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    inject_status(&env, &client.address, id, ContractStatus::Completed);

    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, Error::InvalidState);
}

/// Release on a Cancelled contract → InvalidState.
#[test]
fn release_on_cancelled_contract_fails_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    inject_status(&env, &client.address, id, ContractStatus::Cancelled);

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, Error::InvalidState);
}

/// Approve on a Cancelled contract → InvalidState.
#[test]
fn approve_on_cancelled_contract_fails_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    inject_status(&env, &client.address, id, ContractStatus::Cancelled);

    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, Error::InvalidState);
}

// ===========================================================================
// Section 8 — Wrong-party approval does not unlock release
//
// A wrong-party approval call is rejected at the approve step. Consequently
// no valid approval is stored, and the subsequent release attempt also fails
// with InsufficientApprovals. This verifies the two-step guard works end-to-end.
// ===========================================================================

/// ArbiterOnly: client approval rejected; arbiter release then fails InsufficientApprovals.
#[test]
fn arbiter_only_wrong_party_approval_does_not_unlock_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ArbiterOnly,
    );

    // Client tries to approve for ArbiterOnly mode — must be rejected.
    let approve_result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(approve_result, Error::UnauthorizedRole);

    // No valid approval stored, so the arbiter's release attempt also fails.
    let release_result = client.try_release_milestone(&id, &arbiter_addr, &0);
    assert_contract_error(release_result, Error::InsufficientApprovals);
}

/// ClientOnly: arbiter approval rejected; client release then fails InsufficientApprovals.
#[test]
fn client_only_wrong_party_approval_does_not_unlock_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, arbiter_addr) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        Some(&arbiter_addr),
        &ReleaseAuthorization::ClientOnly,
    );

    // Arbiter tries to approve for ClientOnly mode — must be rejected.
    let approve_result = client.try_approve_milestone_release(&id, &arbiter_addr, &0);
    assert_contract_error(approve_result, EscrowError::UnauthorizedRole);

    // No valid approval stored, so client's release attempt also fails.
    let release_result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(release_result, Error::InsufficientApprovals);
}

/// MultiSig: freelancer-only approval does not unlock release even for freelancer caller.
#[test]
fn multisig_single_party_approval_does_not_unlock_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::MultiSig,
    );

    // Only freelancer approves — client has not approved.
    assert!(client.approve_milestone_release(&id, &freelancer_addr, &0));

    // Neither party can release with only one approval.
    assert_contract_error(
        client.try_release_milestone(&id, &freelancer_addr, &0),
        Error::InsufficientApprovals,
    );
    assert_contract_error(
        client.try_release_milestone(&id, &client_addr, &0),
        Error::InsufficientApprovals,
    );
}

// ===========================================================================
// Section 9 — Paused contract blocks both approve and release
//
// While the contract is paused, all mutating entrypoints must fail with
// ContractPaused. This boundary applies to every authorization mode.
// ===========================================================================

/// Paused contract: approve_milestone_release is blocked with ContractPaused.
#[test]
fn paused_contract_blocks_approve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    client.pause();

    let result = client.try_approve_milestone_release(&id, &client_addr, &0);
    assert_contract_error(result, EscrowError::ContractPaused);
}

/// Paused contract: release_milestone is blocked with ContractPaused.
#[test]
fn paused_contract_blocks_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    // Record approval before pausing.
    client.approve_milestone_release(&id, &client_addr, &0);
    client.pause();

    let result = client.try_release_milestone(&id, &client_addr, &0);
    assert_contract_error(result, EscrowError::ContractPaused);
}

/// After unpause, approve and release succeed normally.
#[test]
fn unpause_restores_approve_and_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    client.pause();
    client.unpause();

    // Both operations must succeed after unpause.
    assert!(client.approve_milestone_release(&id, &client_addr, &0));
    assert!(client.release_milestone(&id, &client_addr, &0));
}

// ===========================================================================
// Section 10 — Approval state is fail-closed after failed releases
//
// A failed release attempt (wrong caller, missing approvals, wrong state)
// must leave contract accounting untouched.
// ===========================================================================

/// Failed release (UnauthorizedRole) leaves released_amount and status unchanged.
#[test]
fn failed_release_unauthorized_leaves_state_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );
    client.approve_milestone_release(&id, &client_addr, &0);

    let before = client.get_contract(&id);
    let attacker = Address::generate(&env);
    let _ = client.try_release_milestone(&id, &attacker, &0);
    let after = client.get_contract(&id);

    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
}

/// Failed release (InsufficientApprovals) leaves released_amount and status unchanged.
#[test]
fn failed_release_insufficient_approvals_leaves_state_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    let client = new_client(&env);
    let (client_addr, freelancer_addr, _) = participants(&env);

    let id = create_funded(
        &env,
        &client,
        &client_addr,
        &freelancer_addr,
        None,
        &ReleaseAuthorization::ClientOnly,
    );

    // No approval recorded.
    let before = client.get_contract(&id);
    let _ = client.try_release_milestone(&id, &client_addr, &0);
    let after = client.get_contract(&id);

    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
}
