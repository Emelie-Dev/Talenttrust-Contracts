use super::MILESTONE_ONE;
use crate::{ContractStatus, DisputeResolution, EscrowError, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, vec, Address};

fn setup(arbiter: bool) -> super::EscrowFixture {
    let builder = super::EscrowFixtureBuilder::new();
    let client = Address::generate(builder.env());
    let freelancer = Address::generate(builder.env());
    let arbiter_addr = if arbiter {
        Some(Address::generate(builder.env()))
    } else {
        None
    };
    builder
        .with_participants(client, freelancer, arbiter_addr)
        .funded()
        .build()
}

#[test]
fn admin_can_rollback_completed_contract() {
    let fixture = setup(false);
    let contract_id = fixture.escrow_id;
    let client = fixture.client.clone();
    let admin = fixture.admin.clone();
    let escrow = fixture.escrow();

    for i in 0..3u32 {
        escrow.approve_milestone_release(&contract_id, &client, &i);
        escrow.release_milestone(&contract_id, &client, &i);
    }
    assert_eq!(
        escrow.get_contract(&contract_id).status,
        ContractStatus::Completed
    );

    escrow.finalize_contract(&contract_id, &client);
    assert!(escrow.get_finalization_record(&contract_id).is_some());

    let before = escrow.get_contract(&contract_id);
    assert!(escrow.rollback_contract(&admin, &contract_id));
    let after = escrow.get_contract(&contract_id);

    assert!(escrow.get_finalization_record(&contract_id).is_none());
    assert_eq!(after.status, ContractStatus::Completed);
    assert_eq!(after.funded_amount, before.funded_amount);
    assert_eq!(after.released_amount, before.released_amount);
    assert_eq!(after.refunded_amount, before.refunded_amount);
}

#[test]
fn admin_can_rollback_disputed_contract_and_resolve_afterwards() {
    let fixture = setup(true);
    let contract_id = fixture.escrow_id;
    let client = fixture.client.clone();
    let admin = fixture.admin.clone();
    let arbiter = fixture.arbiter.clone().unwrap();
    let escrow = fixture.escrow();

    escrow.raise_dispute(&contract_id, &client);
    assert_eq!(
        escrow.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );

    escrow.finalize_contract(&contract_id, &client);
    assert!(escrow.get_finalization_record(&contract_id).is_some());

    assert!(escrow.rollback_contract(&admin, &contract_id));

    assert!(escrow.get_finalization_record(&contract_id).is_none());
    assert_eq!(
        escrow.get_contract(&contract_id).status,
        ContractStatus::Disputed
    );

    // After rollback the arbiter can resolve the dispute again.
    escrow.resolve_dispute(&contract_id, &arbiter, &DisputeResolution::FullRefund);
    assert_eq!(
        escrow.get_contract(&contract_id).status,
        ContractStatus::Refunded
    );
}

#[test]
fn non_admin_cannot_rollback() {
    let fixture = setup(false);
    let contract_id = fixture.escrow_id;
    let client = fixture.client.clone();
    let escrow = fixture.escrow();

    for i in 0..3u32 {
        escrow.approve_milestone_release(&contract_id, &client, &i);
        escrow.release_milestone(&contract_id, &client, &i);
    }
    escrow.finalize_contract(&contract_id, &client);

    let result = escrow.try_rollback_contract(&client, &contract_id);
    super::assert_contract_error(result, EscrowError::UnauthorizedRole);
}

#[test]
fn rollback_rejected_when_not_finalized() {
    let fixture = setup(false);
    let contract_id = fixture.escrow_id;
    let client = fixture.client.clone();
    let admin = fixture.admin.clone();
    let escrow = fixture.escrow();

    for i in 0..3u32 {
        escrow.approve_milestone_release(&contract_id, &client, &i);
        escrow.release_milestone(&contract_id, &client, &i);
    }
    // Contract is Completed but not finalized.

    let result = escrow.try_rollback_contract(&admin, &contract_id);
    super::assert_contract_error(result, EscrowError::RollbackNotAllowed);
}

#[test]
fn rollback_rejected_for_created_contract() {
    let fixture = setup(false);
    let new_client = Address::generate(&fixture.env);
    let new_freelancer = Address::generate(&fixture.env);
    let milestones = vec![&fixture.env, MILESTONE_ONE];
    let admin = fixture.admin.clone();
    let escrow = fixture.escrow();
    let contract_id = escrow.create_contract(
        &new_client,
        &new_freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = escrow.try_rollback_contract(&admin, &contract_id);
    super::assert_contract_error(result, EscrowError::RollbackNotAllowed);
}
