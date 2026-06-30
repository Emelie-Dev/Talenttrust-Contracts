use soroban_sdk::{testutils::Address as _, testutils::Events, vec, Address, Env};

use super::{
    assert_contract_error, complete_contract, create_contract, register_client,
    total_milestone_amount,
};
use crate::{ContractStatus, Error};
#[test]
fn refund_succeeds_on_funded_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer, contract_id) = create_contract(&env, &client);

    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    let refund_ids = vec![&env, 1_u32];
    let refunded = client.refund_unreleased_milestones(&contract_id, &refund_ids);
    assert_eq!(refunded, 400_0000000_i128);

    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::Funded);
}

#[test]
fn rejects_refund_on_cancelled_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer, contract_id) = create_contract(&env, &client);

    assert!(client.cancel_contract(&contract_id, &client_addr));

    let refund_ids = vec![&env, 0_u32];
    assert_contract_error(
        client.try_refund_unreleased_milestones(&contract_id, &refund_ids),
        Error::InvalidState,
    );
}

#[test]
fn rejects_refund_on_completed_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _freelancer, contract_id) = complete_contract(&env, &client);

    let refund_ids = vec![&env, 0_u32];
    assert_contract_error(
        client.try_refund_unreleased_milestones(&contract_id, &refund_ids),
        Error::InvalidState,
    );
}

#[test]
fn rejects_refund_on_finalized_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer, contract_id) = complete_contract(&env, &client);

    assert!(client.finalize_contract(&contract_id, &client_addr));

    let refund_ids = vec![&env, 0_u32];
    assert_contract_error(
        client.try_refund_unreleased_milestones(&contract_id, &refund_ids),
        Error::AlreadyFinalized,
    );
}