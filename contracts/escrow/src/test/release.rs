use soroban_sdk::{symbol_short, testutils::Address as _, vec, Address, Env, String};

use super::{
    assert_contract_error, create_contract, register_client, total_milestone_amount,
    MILESTONE_ONE,
};
use crate::{ContractStatus, Error, EscrowError, ReleaseAuthorization};

fn evidence(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

// ---------------------------------------------------------------------------
// Release flow tests
// ---------------------------------------------------------------------------

#[test]
fn releases_funded_milestones_and_completes_when_all_released() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Funded);
    assert_eq!(contract.released_amount, MILESTONE_ONE);

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    assert!(client.release_milestone(&contract_id, &client_addr, &1));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &2));
    assert!(client.release_milestone(&contract_id, &client_addr, &2));

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Completed);
    assert_eq!(contract.released_amount, total_milestone_amount());
    assert_eq!(client.get_refundable_balance(&contract_id), 0);
}

#[test]
fn rejects_release_without_sufficient_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    assert_contract_error(result, EscrowError::InsufficientFunds);
}

#[test]
fn rejects_release_of_invalid_milestone_index() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    let result = client.try_release_milestone(&contract_id, &client_addr, &99);
    assert_contract_error(result, EscrowError::InvalidMilestone);
}

#[test]
fn rejects_releasing_refunded_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    client.refund_unreleased_milestones(&contract_id, &vec![&env, 1_u32]);

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &1));
    let result = client.try_release_milestone(&contract_id, &client_addr, &1);
    assert_contract_error(result, EscrowError::AlreadyRefunded);
}

#[test]
fn rejects_releasing_same_milestone_twice() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));
    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    assert!(client.release_milestone(&contract_id, &client_addr, &0));

    assert!(client.approve_milestone_release(&contract_id, &client_addr, &0));
    let result = client.try_release_milestone(&contract_id, &client_addr, &0);
    assert_contract_error(result, EscrowError::AlreadyReleased);
}

// ---------------------------------------------------------------------------
// submit_work_evidence tests
// ---------------------------------------------------------------------------

#[test]
fn work_evidence_stored_on_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let ev = evidence(&env, "ipfs://QmExampleCid");
    assert!(escrow.submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev));

    let milestones = escrow.get_milestones(&contract_id);
    assert_eq!(milestones.get(0).unwrap().work_evidence, Some(ev));
}

#[test]
fn work_evidence_can_be_overwritten_before_release() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let v1 = evidence(&env, "ipfs://first");
    let v2 = evidence(&env, "ipfs://second");
    assert!(escrow.submit_work_evidence(&contract_id, &freelancer_addr, &0, &v1));
    assert!(escrow.submit_work_evidence(&contract_id, &freelancer_addr, &0, &v2));

    let milestones = escrow.get_milestones(&contract_id);
    assert_eq!(milestones.get(0).unwrap().work_evidence, Some(v2));
}

#[test]
fn work_evidence_does_not_affect_other_milestones() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let ev = evidence(&env, "ipfs://QmOnlyMilestone0");
    assert!(escrow.submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev));

    let milestones = escrow.get_milestones(&contract_id);
    assert!(milestones.get(1).unwrap().work_evidence.is_none());
    assert!(milestones.get(2).unwrap().work_evidence.is_none());
}

#[test]
fn work_evidence_emits_evidence_event() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let ev = evidence(&env, "ipfs://QmTest");
    assert!(escrow.submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev));

    let events = env.events().all();
    assert!(events.iter().any(|e| e.0 == symbol_short!("evidence")));
}

#[test]
fn work_evidence_rejects_non_freelancer_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let ev = evidence(&env, "ipfs://QmTest");
    // client is not the freelancer
    let result = escrow.try_submit_work_evidence(&contract_id, &client_addr, &0, &ev);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn work_evidence_rejects_stranger() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let stranger = Address::generate(&env);
    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&contract_id, &stranger, &0, &ev);
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn work_evidence_rejects_unfunded_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (_client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    // no deposit — contract stays in Created status

    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev);
    assert_contract_error(result, Error::InvalidState);
}

#[test]
fn work_evidence_rejects_released_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());
    escrow.approve_milestone_release(&contract_id, &client_addr, &0);
    escrow.release_milestone(&contract_id, &client_addr, &0);

    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev);
    assert_contract_error(result, Error::MilestoneAlreadyReleased);
}

#[test]
fn work_evidence_rejects_refunded_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());
    escrow.refund_unreleased_milestones(&contract_id, &vec![&env, 0_u32]);

    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev);
    assert_contract_error(result, Error::AlreadyRefunded);
}

#[test]
fn work_evidence_rejects_out_of_bounds_index() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &99, &ev);
    assert_contract_error(result, Error::IndexOutOfBounds);
}

#[test]
fn work_evidence_rejects_oversized_string() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    // 257 chars > 256-byte limit
    let ev = String::from_str(&env, &"a".repeat(257));
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev);
    assert_contract_error(result, EscrowError::EvidenceTooLong);
}

#[test]
fn work_evidence_accepts_exactly_256_byte_string() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    let ev = String::from_str(&env, &"a".repeat(256));
    assert!(escrow.submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev));
}

#[test]
fn work_evidence_rejects_paused_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);

    let admin = Address::generate(&env);
    escrow.initialize(&admin);

    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());
    escrow.pause();

    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev);
    assert_contract_error(result, EscrowError::ContractPaused);
}

#[test]
fn work_evidence_rejects_finalized_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &escrow);
    escrow.deposit_funds(&contract_id, &client_addr, &total_milestone_amount());

    // Release all milestones → Completed → finalize
    escrow.approve_milestone_release(&contract_id, &client_addr, &0);
    escrow.release_milestone(&contract_id, &client_addr, &0);
    escrow.approve_milestone_release(&contract_id, &client_addr, &1);
    escrow.release_milestone(&contract_id, &client_addr, &1);
    escrow.approve_milestone_release(&contract_id, &client_addr, &2);
    escrow.release_milestone(&contract_id, &client_addr, &2);
    escrow.finalize_contract(&contract_id, &client_addr);

    let ev = evidence(&env, "ipfs://QmAfterFinalize");
    let result = escrow.try_submit_work_evidence(&contract_id, &freelancer_addr, &0, &ev);
    assert_contract_error(result, EscrowError::AlreadyFinalized);
}

#[test]
fn work_evidence_rejects_unknown_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let freelancer = Address::generate(&env);

    let ev = evidence(&env, "ipfs://QmTest");
    let result = escrow.try_submit_work_evidence(&9999, &freelancer, &0, &ev);
    assert_contract_error(result, Error::ContractNotFound);
}
