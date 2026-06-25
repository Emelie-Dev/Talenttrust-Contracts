use soroban_sdk::vec;

use super::{
    assert_contract_error, assert_contract_state, assert_milestone_flags, create_client,
    create_default_contract, setup, total_milestone_amount, MILESTONE_ONE, MILESTONE_THREE,
    MILESTONE_TWO,
};
use crate::{types::Error, ContractStatus};

// ── Single-milestone release (existing tests) ───────────────────────────────

/// Tests that milestones can be released sequentially and contract completes when all are released.
///
/// # Security
/// - Validates authorization checks for release
/// - Ensures released_amount tracking is accurate
/// - Verifies state transition to Completed
/// - Confirms refundable balance calculation
#[test]
fn releases_funded_milestones_and_completes_when_all_are_released() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &1_200_0000000_i128));

    // Approve and release first milestone
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    let contract = client.get_contract(&contract_id);
    assert_contract_state(
        contract,
        ContractStatus::Funded,
        1_200_0000000_i128,
        200_0000000_i128,
        0,
    );
    assert_milestone_flags(client.get_milestones(&contract_id), 0, true, false);
    assert_eq!(
        client.get_refundable_balance(&contract_id),
        1_000_0000000_i128
    );

    // Approve and release remaining milestones
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    assert!(client.release_milestone(&contract_id, &client_addr, &1));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &2));
    assert!(client.release_milestone(&contract_id, &client_addr, &2));

    let contract = client.get_contract(&contract_id);
    assert_contract_state(
        contract,
        ContractStatus::Completed,
        1_200_0000000_i128,
        1_200_0000000_i128,
        0,
    );
    assert_eq!(client.get_refundable_balance(&contract_id), 0);
}

/// Tests that release is rejected when insufficient funds are available.
///
/// # Security
/// - Prevents overdraft attacks
/// - Validates balance checks before release
#[test]
#[should_panic]
fn rejects_release_without_sufficient_balance() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &100_0000000_i128));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    client.release_milestone(&contract_id, &client_addr, &0);
}

/// Tests that release of invalid milestone index is rejected.
///
/// # Security
/// - Prevents out-of-bounds access
/// - Validates milestone index bounds
#[test]
#[should_panic]
fn rejects_release_of_invalid_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &1_200_0000000_i128));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &3));
    client.release_milestone(&contract_id, &client_addr, &3);
}

/// Tests that releasing a refunded milestone is rejected.
///
/// # Security
/// - Prevents double-spending
/// - Validates milestone state before release
#[test]
#[should_panic]
fn rejects_releasing_refunded_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &1_200_0000000_i128));
    let refund_ids = vec![&env, 1_u32];
    client.refund_unreleased_milestones(&contract_id, &refund_ids);

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    client.release_milestone(&contract_id, &client_addr, &1);
}

/// Tests that releasing the same milestone twice is rejected.
///
/// # Security
/// - Prevents double-spending
/// - Validates milestone released flag
#[test]
#[should_panic]
fn rejects_releasing_same_milestone_twice() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &1_200_0000000_i128));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    client.release_milestone(&contract_id, &client_addr, &0);
}

// ── Batch release ────────────────────────────────────────────────────────────

/// Full-batch happy path: all three milestones released in a single call.
///
/// Verifies:
/// - Return value equals aggregate amount
/// - released_amount and status are updated atomically
/// - All milestone flags are set
/// - Refundable balance drops to zero
#[test]
fn batch_releases_all_milestones_atomically_and_completes_contract() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    let total = total_milestone_amount();

    assert!(client.deposit_funds(&contract_id, &client_addr, &total));

    // Approve all three milestones.
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &2));

    let released =
        client.release_milestones_batch(&contract_id, &client_addr, &vec![&env, 0_u32, 1_u32, 2_u32]);
    assert_eq!(released, total);

    let contract = client.get_contract(&contract_id);
    assert_contract_state(
        contract,
        ContractStatus::Completed,
        total,
        total,
        0,
    );
    assert_milestone_flags(client.get_milestones(&contract_id), 0, true, false);
    assert_milestone_flags(client.get_milestones(&contract_id), 1, true, false);
    assert_milestone_flags(client.get_milestones(&contract_id), 2, true, false);
    assert_eq!(client.get_refundable_balance(&contract_id), 0);
}

/// Partial-batch: release milestones 0 and 2, leave 1 unreleased.
///
/// Verifies:
/// - Only selected milestones are marked released
/// - released_amount is exactly the sum of the two milestones
/// - Contract remains in Funded state (not all settled)
#[test]
fn batch_releases_subset_of_milestones_and_contract_remains_funded() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &2));

    let released =
        client.release_milestones_batch(&contract_id, &client_addr, &vec![&env, 0_u32, 2_u32]);
    assert_eq!(released, MILESTONE_ONE + MILESTONE_THREE);

    let contract = client.get_contract(&contract_id);
    assert_contract_state(
        contract,
        ContractStatus::Funded,
        total_milestone_amount(),
        MILESTONE_ONE + MILESTONE_THREE,
        0,
    );
    assert_milestone_flags(client.get_milestones(&contract_id), 0, true, false);
    assert_milestone_flags(client.get_milestones(&contract_id), 1, false, false);
    assert_milestone_flags(client.get_milestones(&contract_id), 2, true, false);
}

/// Batch with a single milestone is equivalent to release_milestone.
#[test]
fn batch_of_one_mirrors_single_release() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));

    let released =
        client.release_milestones_batch(&contract_id, &client_addr, &vec![&env, 1_u32]);
    assert_eq!(released, MILESTONE_TWO);

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.released_amount, MILESTONE_TWO);
    assert_milestone_flags(client.get_milestones(&contract_id), 1, true, false);
}

/// Batch with mixed already-released/not triggers complete status when all settled.
#[test]
fn batch_completes_contract_when_last_milestone_released() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    // Release first two individually.
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    assert!(client.release_milestone(&contract_id, &client_addr, &1));

    // Batch-release the last one.
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &2));
    let released =
        client.release_milestones_batch(&contract_id, &client_addr, &vec![&env, 2_u32]);
    assert_eq!(released, MILESTONE_THREE);

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Completed);
}

// ── Batch rejection / no partial state ──────────────────────────────────────

/// Empty batch is rejected.
///
/// # Security — no partial state on rejection
#[test]
fn batch_rejects_empty_index_list() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    let result = client.try_release_milestones_batch(
        &contract_id,
        &client_addr,
        &vec![&env],
    );
    assert_contract_error(result, Error::EmptyBatchRelease);
}

/// Duplicate indices in batch are rejected before any mutation.
///
/// # Security — no partial state on rejection
#[test]
fn batch_rejects_duplicate_indices() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));

    let result = client.try_release_milestones_batch(
        &contract_id,
        &client_addr,
        &vec![&env, 0_u32, 0_u32],
    );
    assert_contract_error(result, Error::DuplicateMilestoneInBatch);

    // No state should have changed.
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.released_amount, 0);
}

/// Batch including an already-released milestone is rejected atomically —
/// other valid milestones in the batch must NOT be released.
///
/// # Security — no partial state on rejection
#[test]
fn batch_rejects_if_any_milestone_already_released_and_leaves_no_partial_state() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    // Release milestone 0 individually.
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));

    // Approve milestone 1 so approval check is not the failure reason.
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));

    // Batch includes already-released 0 and valid 1.
    let result = client.try_release_milestones_batch(
        &contract_id,
        &client_addr,
        &vec![&env, 1_u32, 0_u32],
    );
    assert_contract_error(result, Error::MilestoneAlreadyReleased);

    // Milestone 1 must still be unreleased.
    assert_milestone_flags(client.get_milestones(&contract_id), 1, false, false);
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.released_amount, MILESTONE_ONE);
}

/// Batch where one milestone has no approval fails with InsufficientApprovals,
/// and no other milestone is released.
///
/// # Security — no partial state on rejection
#[test]
fn batch_rejects_if_any_milestone_lacks_approval_and_leaves_no_partial_state() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    // Approve only milestone 0.
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    // Milestone 1 has no approval.

    let result = client.try_release_milestones_batch(
        &contract_id,
        &client_addr,
        &vec![&env, 0_u32, 1_u32],
    );
    assert_contract_error(result, Error::InsufficientApprovals);

    // Neither milestone should be released.
    assert_milestone_flags(client.get_milestones(&contract_id), 0, false, false);
    assert_milestone_flags(client.get_milestones(&contract_id), 1, false, false);
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.released_amount, 0);
}

/// Batch is rejected when aggregate amount exceeds available balance.
///
/// # Security — no partial state on rejection
#[test]
fn batch_rejects_when_aggregate_amount_exceeds_available_balance() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    // Deposit only enough for milestone 0.
    assert!(client.deposit_funds(&contract_id, &client_addr, &MILESTONE_ONE));

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));

    // Batch requests both 0 and 1 but only 0 is funded.
    let result = client.try_release_milestones_batch(
        &contract_id,
        &client_addr,
        &vec![&env, 0_u32, 1_u32],
    );
    assert_contract_error(result, Error::InsufficientFunds);

    // No state mutated.
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.released_amount, 0);
}

/// Batch out-of-bounds index is rejected.
#[test]
fn batch_rejects_out_of_bounds_index() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    let result = client.try_release_milestones_batch(
        &contract_id,
        &client_addr,
        &vec![&env, 99_u32],
    );
    assert_contract_error(result, Error::IndexOutOfBounds);
}

/// Unauthorized caller is rejected before any state mutation.
#[test]
fn batch_rejects_unauthorized_caller() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));

    let result = client.try_release_milestones_batch(
        &contract_id,
        &freelancer_addr, // freelancer is not authorized under ClientOnly
        &vec![&env, 0_u32],
    );
    assert_contract_error(result, Error::UnauthorizedRole);
}
