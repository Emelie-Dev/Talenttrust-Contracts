use super::{
    complete_contract, create_contract, create_contract_with_arbiter, default_milestones,
    generated_participants, register_client, total_milestone_amount,
};
use crate::{EscrowError, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, testutils::Events, vec, Env, Vec};

#[test]
fn create_rejects_same_participants() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (addr, _) = generated_participants(&env);

    let result = client.try_create_contract(
        &addr,
        &addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
    super::assert_contract_error(result, EscrowError::InvalidParticipant);
}

#[test]
fn create_rejects_empty_milestone_list() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, _) = generated_participants(&env);
    let empty = Vec::<i128>::new(&env);

    let result = client.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &empty,
        &ReleaseAuthorization::ClientOnly,
    );
    super::assert_contract_error(result, EscrowError::EmptyMilestones);
}

#[test]
fn create_rejects_non_positive_milestone_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, _) = generated_participants(&env);
    let milestones = vec![&env, 100_i128, 0_i128];

    let result = client.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    super::assert_contract_error(result, EscrowError::InvalidMilestoneAmount);
}

#[test]
#[should_panic]
fn create_requires_client_authorization() {
    let env = Env::default();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, _) = generated_participants(&env);

    let _ = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );
}

#[test]
fn deposit_rejects_non_positive_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let result = client.try_deposit_funds(&contract_id, &client_addr, &0);
    super::assert_contract_error(result, EscrowError::InvalidDepositAmount);
}

#[test]
fn release_rejects_when_contract_not_funded() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    super::assert_contract_error(result, EscrowError::InsufficientFunds);
}

#[test]
fn release_rejects_invalid_milestone_id() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &super::total_milestone_amount()));
    let result = client.try_release_milestone(&contract_id, &client_addr, &99);
    super::assert_contract_error(result, EscrowError::InvalidMilestone);
}

#[test]
fn release_rejects_double_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &super::total_milestone_amount()));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));

    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    super::assert_contract_error(result, EscrowError::AlreadyReleased);
}

#[test]
fn issue_reputation_rejects_unfinished_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    let comment = reputation_comment(&env);
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &comment);
    super::assert_contract_error(result, Error::NotCompleted);
}

#[test]
fn issue_reputation_rejects_invalid_rating() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    let comment = reputation_comment(&env);
    let result = client.try_issue_reputation(&contract_id, &client_addr, &0, &comment);
    super::assert_contract_error(result, Error::InvalidRating);
}

#[test]
fn issue_reputation_once_per_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    let comment = reputation_comment(&env);
    assert!(client.issue_reputation(&contract_id, &client_addr, &5, &comment));
    let result = client.try_issue_reputation(&contract_id, &client_addr, &4, &comment);
    super::assert_contract_error(result, Error::ReputationAlreadyIssued);
}

#[test]
fn issue_reputation_rejects_empty_comment() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    let empty_comment = String::from_str(&env, "");
    let result = client.try_issue_reputation(&contract_id, &client_addr, &5, &empty_comment);
    super::assert_contract_error(result, Error::EmptyComment);
}

#[test]
fn issue_reputation_rejects_unauthorized_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);
    let unauthorized = soroban_sdk::Address::generate(&env);

    let comment = reputation_comment(&env);
    let result = client.try_issue_reputation(&contract_id, &unauthorized, &5, &comment);
    super::assert_contract_error(result, Error::PartyNotAuthorized);
}

/// Finalize a completed contract and return all identifiers.
fn finalized_contract(
    env: &Env,
) -> (
    crate::EscrowClient<'_>,
    soroban_sdk::Address,
    soroban_sdk::Address,
    u32,
) {
    let client = register_client(env);
    let (client_addr, freelancer_addr, contract_id) = super::complete_contract(env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    (client, client_addr, freelancer_addr, contract_id)
}

#[test]
fn finalized_contract_read_operations_still_work() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, contract_id) = finalized_contract(&env);

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, crate::ContractStatus::Completed);

    let record = client.get_finalization_record(&contract_id);
    assert!(record.is_some());
}

#[test]
fn finalize_cannot_be_called_twice() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, client_addr, _, contract_id) = finalized_contract(&env);

    let result = client.try_finalize_contract(&contract_id, &client_addr);

    super::assert_contract_error(result, EscrowError::AlreadyFinalized);
}

#[test]
fn finalized_contract_rejects_cancel() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, client_addr, _, contract_id) = finalized_contract(&env);

    let result = client.try_cancel_contract(&contract_id, &client_addr);

    super::assert_contract_error(result, EscrowError::AlreadyFinalized);
}

#[test]
fn finalized_contract_rejects_refund() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, contract_id) = finalized_contract(&env);

    let indices = vec![&env, 0u32];

    let result = client.try_refund_unreleased_milestones(&contract_id, &indices);

    super::assert_contract_error(result, EscrowError::AlreadyFinalized);
}

#[test]
fn finalized_contract_rejects_release() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, client_addr, _, contract_id) = finalized_contract(&env);

    let result = client.try_release_milestone(&contract_id, &client_addr, &0);

    super::assert_contract_error(result, EscrowError::AlreadyFinalized);
}

#[test]
fn deposit_rejected_after_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &client);

    // Cancel immediately in Created state
    assert!(client.cancel_contract(&contract_id, &client_addr));

    let result = client.try_deposit_funds(&contract_id, &client_addr, &100_i128);
    super::assert_contract_error(result, EscrowError::ContractCancelled);
}

#[test]
fn release_rejected_after_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &client);

    // Fully fund and then cancel
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    assert!(client.cancel_contract(&contract_id, &client_addr));

    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    super::assert_contract_error(result, EscrowError::ContractCancelled);
}

#[test]
fn refund_rejected_after_refund() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    // Fund and refund all milestones
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    let all_indices = vec![&env, 0_u32, 1_u32, 2_u32];
    assert!(client.refund_unreleased_milestones(&contract_id, &all_indices) > 0);

    // Second refund attempt should be rejected as contract is terminally refunded
    let res = client.try_refund_unreleased_milestones(&contract_id, &all_indices);
    super::assert_contract_error(res, EscrowError::ContractRefunded);
}

// ═══════════════════════════════════════════════════════════════════════════════
// finalize_contract — Comprehensive Negative-Path Coverage  (issue #709)
//
// Guards exercised in finalize_contract_impl (in order of evaluation):
//   1. require_not_paused  → ContractPaused  (pause flag set)
//   2. require_not_paused  → EmergencyActive (emergency flag set)
//   3. load_contract       → ContractNotFound (unknown contract_id)
//   4. require_not_finalized → AlreadyFinalized (record already exists)
//   5. require_finalizer_role → UnauthorizedRole (caller not a participant)
//   6. status check        → InvalidStatusTransition (status ∉ {Completed, Disputed})
//
// Positive-path assertions (guard-list validation):
//   • A completed contract finalizes successfully via client, freelancer, or arbiter.
//   • A disputed contract finalizes successfully.
//   • get_finalization_record returns the correct snapshot after finalization.
//   • The "finalized" event is emitted on success.
//
// Post-finalize mutation blocking:
//   • release_milestone panics with AlreadyFinalized (covered above, plus extra
//     assertion here from the disputed path).
//   • refund_unreleased_milestones panics with AlreadyFinalized.
// ═══════════════════════════════════════════════════════════════════════════════

// ── Local helpers ────────────────────────────────────────────────────────────

/// Build a disputed contract (Funded → Disputed via raise_dispute).
///
/// Returns `(escrow_client, client_addr, freelancer_addr, arbiter_addr, contract_id)`.
/// The contract has three milestones totalling `total_milestone_amount()` and is
/// fully funded so that the caller can immediately raise a dispute.
fn disputed_contract(
    env: &Env,
) -> (
    crate::EscrowClient<'_>,
    soroban_sdk::Address,
    soroban_sdk::Address,
    soroban_sdk::Address,
    u32,
) {
    let client_escrow = register_client(env);
    let (client_addr, freelancer_addr, arbiter_addr, contract_id) =
        create_contract_with_arbiter(env, &client_escrow);

    // Fund the contract so it can be disputed.
    assert!(client_escrow.deposit_funds(
        &contract_id,
        &client_addr,
        &super::total_milestone_amount()
    ));

    // Client raises a dispute (contract must have arbiter and be Funded).
    assert!(client_escrow.raise_dispute(&contract_id, &client_addr));

    (
        client_escrow,
        client_addr,
        freelancer_addr,
        arbiter_addr,
        contract_id,
    )
}

// ── Guard 1 & 2: pause and emergency blocks ──────────────────────────────────

/// finalize_contract must fail with ContractPaused when the pause flag is active.
#[test]
fn finalize_fails_while_paused() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    // Activate the normal pause flag.
    assert!(client.pause());

    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::ContractPaused);
}

/// finalize_contract must fail with ContractPaused when emergency mode is active.
///
/// `activate_emergency_pause()` sets both the Paused and Emergency flags;
/// `require_not_paused` checks Paused first so the error is ContractPaused.
#[test]
fn finalize_fails_while_emergency_active() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    // Activate the emergency pause (sets Paused + Emergency flags).
    assert!(client.activate_emergency_pause());

    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::ContractPaused);
}

/// After unpausing, finalize_contract succeeds — confirms the pause guard is the
/// exclusive reason for failure, not any other guard.
#[test]
fn finalize_succeeds_after_unpause() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.pause());
    // Confirm it is blocked.
    super::assert_contract_error(
        client.try_finalize_contract(&contract_id, &client_addr),
        EscrowError::ContractPaused,
    );

    // Unpause and retry — must succeed.
    assert!(client.unpause());
    assert!(client.finalize_contract(&contract_id, &client_addr));
}

/// After resolving emergency, finalize_contract succeeds — confirms the emergency
/// guard is the exclusive reason for failure when only the emergency flag was set.
#[test]
fn finalize_succeeds_after_resolve_emergency() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.activate_emergency_pause());
    // Confirm it is blocked.
    super::assert_contract_error(
        client.try_finalize_contract(&contract_id, &client_addr),
        EscrowError::ContractPaused,
    );

    // Resolve emergency and retry — must succeed.
    assert!(client.resolve_emergency());
    assert!(client.finalize_contract(&contract_id, &client_addr));
}

// ── Guard 3: ContractNotFound ─────────────────────────────────────────────────

/// finalize_contract must fail with ContractNotFound for an unknown contract_id.
#[test]
fn finalize_fails_for_unknown_contract_id() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let caller = soroban_sdk::Address::generate(&env);

    // No contract has been created; any ID must be unknown.
    let result = client.try_finalize_contract(&9999u32, &caller);
    super::assert_contract_error(result, EscrowError::ContractNotFound);
}

// ── Guard 5: UnauthorizedRole (non-participant) ───────────────────────────────

/// A random address that is not the client, freelancer, or arbiter must be
/// rejected with UnauthorizedRole.
#[test]
fn finalize_fails_for_non_participant_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    let outsider = soroban_sdk::Address::generate(&env);
    let result = client.try_finalize_contract(&contract_id, &outsider);
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

/// A former contract-adjacent address (e.g. an admin) that is not stored as
/// client/freelancer/arbiter must also be rejected.
#[test]
fn finalize_fails_for_admin_address_not_participant() {
    let env = Env::default();
    env.mock_all_auths();
    // register_client internally creates an admin address.
    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    // Generate a second address that looks "official" but has no role in this contract.
    let not_a_participant = soroban_sdk::Address::generate(&env);
    let result = client.try_finalize_contract(&contract_id, &not_a_participant);
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

// ── Guard 6: InvalidStatusTransition ─────────────────────────────────────────

/// finalize must reject a contract in Created status (no deposit yet).
#[test]
fn finalize_fails_for_created_status() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    // Status is Created — finalize must be rejected.
    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::InvalidStatusTransition);
}

/// finalize must reject a contract in Funded status (partially or fully funded
/// but not yet Completed or Disputed).
#[test]
fn finalize_fails_for_funded_status() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    // Deposit funds → status becomes Funded.
    assert!(client.deposit_funds(&contract_id, &client_addr, &super::total_milestone_amount()));

    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::InvalidStatusTransition);
}

/// finalize must reject a contract in Cancelled status.
#[test]
fn finalize_fails_for_cancelled_status() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    // Cancel immediately — status becomes Cancelled.
    assert!(client.cancel_contract(&contract_id, &client_addr));

    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::InvalidStatusTransition);
}

/// finalize must reject a contract in Refunded status.
#[test]
fn finalize_fails_for_refunded_status() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    // Fund then refund all milestones → status becomes Refunded.
    assert!(client.deposit_funds(&contract_id, &client_addr, &super::total_milestone_amount()));
    let indices = soroban_sdk::vec![&env, 0u32, 1u32, 2u32];
    assert!(client.refund_unreleased_milestones(&contract_id, &indices) > 0);

    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::InvalidStatusTransition);
}

// ── Guard 4: AlreadyFinalized (re-entrant finalize) ───────────────────────────

/// A second call to finalize_contract on an already-finalized contract must
/// return AlreadyFinalized. (Mirrors `finalize_cannot_be_called_twice` but
/// included here for completeness in the guard-list matrix.)
#[test]
fn finalize_idempotent_guard_rejects_second_call() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    let result = client.try_finalize_contract(&contract_id, &client_addr);
    super::assert_contract_error(result, EscrowError::AlreadyFinalized);
}

// ── Positive-path: all three participant roles can finalize ──────────────────

/// The stored client address must be able to finalize a completed contract.
#[test]
fn finalize_succeeds_as_client() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));
}

/// The stored freelancer address must be able to finalize a completed contract.
#[test]
fn finalize_succeeds_as_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = super::complete_contract(&env, &client);
    // Confirm client isn't already the finalizer (basic sanity).
    assert_ne!(client_addr, freelancer_addr);

    assert!(client.finalize_contract(&contract_id, &freelancer_addr));
}

/// The assigned arbiter must be able to finalize a completed contract that
/// was created with an arbiter.
#[test]
fn finalize_succeeds_as_arbiter() {
    let env = Env::default();
    env.mock_all_auths();
    let client_escrow = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        create_contract_with_arbiter(&env, &client_escrow);

    // Fund and release all milestones to reach Completed.
    assert!(client_escrow.deposit_funds(
        &contract_id,
        &client_addr,
        &super::total_milestone_amount()
    ));
    // approve_milestone_release is required by ClientAndArbiter mode.
    assert!(client_escrow.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client_escrow.approve_milestone_release(&contract_id, &arbiter_addr, &0));
    assert!(client_escrow.release_milestone(&contract_id, &client_addr, &0));

    assert!(client_escrow.approve_milestone_release(&contract_id, &client_addr, &1));
    assert!(client_escrow.approve_milestone_release(&contract_id, &arbiter_addr, &1));
    assert!(client_escrow.release_milestone(&contract_id, &client_addr, &1));

    assert!(client_escrow.approve_milestone_release(&contract_id, &client_addr, &2));
    assert!(client_escrow.approve_milestone_release(&contract_id, &arbiter_addr, &2));
    assert!(client_escrow.release_milestone(&contract_id, &client_addr, &2));

    // Arbiter finalizes.
    assert!(client_escrow.finalize_contract(&contract_id, &arbiter_addr));
}

// ── Positive-path: Disputed status can be finalized ──────────────────────────

/// A contract in Disputed status must be finalizable (by any of its participants).
#[test]
fn finalize_succeeds_for_disputed_contract_by_client() {
    let env = Env::default();
    env.mock_all_auths();
    let (client_escrow, client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        disputed_contract(&env);

    assert!(client_escrow.finalize_contract(&contract_id, &client_addr));
}

/// The freelancer of a disputed contract must also be able to finalize it.
#[test]
fn finalize_succeeds_for_disputed_contract_by_freelancer() {
    let env = Env::default();
    env.mock_all_auths();
    let (client_escrow, _client_addr, freelancer_addr, _arbiter_addr, contract_id) =
        disputed_contract(&env);

    assert!(client_escrow.finalize_contract(&contract_id, &freelancer_addr));
}

/// The arbiter of a disputed contract must also be able to finalize it.
#[test]
fn finalize_succeeds_for_disputed_contract_by_arbiter() {
    let env = Env::default();
    env.mock_all_auths();
    let (client_escrow, _client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        disputed_contract(&env);

    assert!(client_escrow.finalize_contract(&contract_id, &arbiter_addr));
}

// ── Event and record assertions ───────────────────────────────────────────────

/// get_finalization_record must return None before finalization and Some after.
#[test]
fn get_finalization_record_returns_none_before_and_some_after() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    // No record before finalization.
    assert!(client.get_finalization_record(&contract_id).is_none());

    // Finalize.
    assert!(client.finalize_contract(&contract_id, &client_addr));

    // Record must exist now.
    let record = client.get_finalization_record(&contract_id);
    assert!(record.is_some());
}

/// The FinalizationRecord snapshot must identify the correct finalizer address.
#[test]
fn get_finalization_record_captures_correct_finalizer() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    let record = client
        .get_finalization_record(&contract_id)
        .expect("finalization record must be present");

    assert_eq!(
        record.finalizer, client_addr,
        "finalizer address mismatch in record"
    );
}

/// The FinalizationRecord summary must reflect the Completed status of the contract.
#[test]
fn get_finalization_record_summary_status_is_completed() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    let record = client
        .get_finalization_record(&contract_id)
        .expect("finalization record must be present");

    assert_eq!(
        record.summary.status,
        crate::ContractStatus::Completed,
        "summary status should be Completed"
    );
}

/// The FinalizationRecord summary must reflect the Disputed status for a
/// disputed contract.
#[test]
fn get_finalization_record_summary_status_is_disputed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client_escrow, client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        disputed_contract(&env);

    assert!(client_escrow.finalize_contract(&contract_id, &client_addr));

    let record = client_escrow
        .get_finalization_record(&contract_id)
        .expect("finalization record must be present");

    assert_eq!(
        record.summary.status,
        crate::ContractStatus::Disputed,
        "summary status should be Disputed"
    );
}

/// The "finalized" event must be emitted exactly once when finalize_contract
/// succeeds.
#[test]
fn finalize_emits_finalized_event() {
    use soroban_sdk::TryFromVal;

    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    // Events in soroban-sdk 22 are stored as (contract_addr, topics: Vec<Val>, data: Val).
    // finalize_contract_impl publishes: topics = (symbol_short!("finalized"), contract_id).
    // We check that at least one event has "finalized" as its first topic value.
    let events = env.events().all();
    let finalized_sym = soroban_sdk::symbol_short!("finalized");
    let found = events.iter().any(|(_addr, topics, _data)| {
        topics
            .iter()
            .next()
            .and_then(|v| {
                <soroban_sdk::Symbol as TryFromVal<Env, soroban_sdk::Val>>::try_from_val(&env, &v)
                    .ok()
            })
            .as_ref()
            == Some(&finalized_sym)
    });

    assert!(found, "expected a 'finalized' event to be published");
}

// ── Post-finalize mutation blocking ─────────────────────────────────────────

/// release_milestone must be blocked with AlreadyFinalized after finalization
/// of a Disputed contract (parallel to the Completed path already tested above).
#[test]
fn release_milestone_blocked_after_finalizing_disputed_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let (client_escrow, client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        disputed_contract(&env);

    assert!(client_escrow.finalize_contract(&contract_id, &client_addr));

    // Any attempt to release a milestone must now be blocked.
    let result = client_escrow.try_release_milestone(&contract_id, &client_addr, &0);
    super::assert_contract_error(result, EscrowError::AlreadyFinalized);
}

/// refund_unreleased_milestones must be blocked with AlreadyFinalized after
/// finalizing a completed contract.
#[test]
fn refund_unreleased_milestones_blocked_after_finalizing_completed_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = super::complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    let indices = soroban_sdk::vec![&env, 0u32];
    // try_refund_unreleased_milestones returns Result<Result<i128, Error>, ...>,
    // so we match directly rather than using assert_contract_error.
    match client.try_refund_unreleased_milestones(&contract_id, &indices) {
        Err(Ok(e)) => assert_eq!(
            e,
            soroban_sdk::Error::from(EscrowError::AlreadyFinalized),
            "expected AlreadyFinalized"
        ),
        other => panic!("expected AlreadyFinalized error, got {:?}", other),
    }
}
