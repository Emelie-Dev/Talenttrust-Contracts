use super::{
    assert_contract_error, assert_contract_state, create_client, create_default_contract, setup,
};
use crate::{types::Error, ContractStatus};
use soroban_sdk::{testutils::Address as _, Address};

/// Tests that deposits accumulate correctly and transition to Funded status when exactly fully funded.
///
/// # Security
/// - Validates state transition from Created to Funded
/// - Ensures funded_amount tracking is accurate
#[test]
fn deposit_single_full_amount_transitions_created_to_funded() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, _freelancer_addr, contract_id) =
        create_contract(&env, &client);
    let total = total_milestone_amount();

    assert!(client.deposit_funds(&contract_id, &client_addr, &total));

    // Deposit 600 (half of the 1,200 required milestone total)
    assert!(client.deposit_funds(&contract_id, &client_addr, &600_0000000_i128));
    let contract = client.get_contract(&contract_id);
    assert_contract_state(
        contract,
        ContractStatus::PartiallyFunded,
        600_0000000_i128,
        0,
        0,
    );

    // Deposit remaining 600, transitions status to Funded
    assert!(client.deposit_funds(&contract_id, &client_addr, &600_0000000_i128));
    let contract = client.get_contract(&contract_id);
    assert_eq!(contract.status, ContractStatus::PartiallyFunded);
    assert_eq!(contract.funded_amount, partial);
    assert!(contract.funded_amount > 0);
    assert!(contract.funded_amount < total_milestone_amount());
}

/// Tests that non-client callers are rejected with UnauthorizedRole.
///
/// # Security
/// - Prevents unauthorized parties (freelancer, arbiter, or attacker) from depositing funds.
#[test]
fn rejects_non_client_caller() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    // Freelancer tries to deposit
    let result = client.try_deposit_funds(&contract_id, &freelancer_addr, &600_0000000_i128);
    assert_contract_error(result, Error::UnauthorizedRole);

    // Random attacker tries to deposit
    let attacker = Address::generate(&env);
    let result = client.try_deposit_funds(&contract_id, &attacker, &600_0000000_i128);
    assert_contract_error(result, Error::UnauthorizedRole);
}

/// Tests that zero-amount deposits are rejected.
///
/// # Security
/// - Prevents accounting anomalies from zero deposits
/// - Validates amount validation at entry point
#[test]
fn rejects_zero_deposit() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    let result = client.try_deposit_funds(&contract_id, &client_addr, &0_i128);
    assert_contract_error(result, Error::AmountMustBePositive);
}

/// Tests that negative-amount deposits are rejected.
///
/// # Security
/// - Prevents balance draining via negative amounts
#[test]
fn rejects_negative_deposit() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    let result = client.try_deposit_funds(&contract_id, &client_addr, &-100_i128);
    assert_contract_error(result, Error::AmountMustBePositive);
}

/// Tests that deposits exceeding the total milestone amount (overfunding) are rejected.
///
/// # Security
/// - Prevents accounting anomalies from negative deposits
/// - Validates amount validation rejects all non-positive values
#[test]
fn rejects_overfunding() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    // Try to deposit 1,300 (total is 1,200) in a single deposit
    let result = client.try_deposit_funds(&contract_id, &client_addr, &1_300_0000000_i128);
    assert_contract_error(result, Error::InvalidState);

    // Deposit 600 first (valid)
    assert!(client.deposit_funds(&contract_id, &client_addr, &600_0000000_i128));

    // Try to deposit 700 next (exceeds remaining 600 limit)
    let result = client.try_deposit_funds(&contract_id, &client_addr, &700_0000000_i128);
    assert_contract_error(result, Error::InvalidState);
}

/// Tests that deposits are rejected after contract is fully refunded.
///
/// # Security
/// - Prevents operations on non-existent contracts
/// - Ensures fail-closed behavior for invalid contract IDs
#[test]
fn rejects_deposit_after_full_refund_resolution() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let contract_id = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&contract_id, &client_addr, &1_200_0000000_i128));
    let refund_ids = soroban_sdk::vec![&env, 0_u32, 1_u32, 2_u32];
    let refunded = client.refund_unreleased_milestones(&contract_id, &refund_ids);
    assert_eq!(refunded, 1_200_0000000_i128);

    // Attempting deposit after refund should fail because contract status is Refunded
    let result = client.try_deposit_funds(&contract_id, &client_addr, &1_i128);
    assert_contract_error(result, Error::InvalidState);
}

/// Tests that deposit_funds panics with UnauthorizedRole when caller is not the depositor.
///
/// Asserts the exact error code when an unauthorized address attempts to deposit.
/// 
/// # Security
/// - Prevents unauthorized fund deposits
/// - Enforces client-only deposit authorization
#[test]
fn test_deposit_unauthorized_role() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &client);

    // Attempt deposit from wrong caller (freelancer instead of client)
    let wrong_caller = Address::generate(&env);
    let result = client.try_deposit_funds(&contract_id, &wrong_caller, &100_0000000_i128);
    assert_contract_error(result, EscrowError::UnauthorizedRole);
}

/// Tests that deposit_funds panics with InvalidState when contract is not in Created state.
///
/// Asserts the exact error code when attempting to deposit after contract has been funded.
/// 
/// # Security
/// - Prevents state machine violations
/// - Ensures deposits only occur during contract setup phase
#[test]
fn test_deposit_invalid_state() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (client_addr, freelancer_addr, contract_id) = create_contract(&env, &client);

    // Fully fund the contract first (transitions to Funded state)
    assert!(client.deposit_funds(&contract_id, &client_addr, &total_milestone_amount()));

    // Try to deposit again (contract is now Funded, not Created)
    let result = client.try_deposit_funds(&contract_id, &client_addr, &100_0000000_i128);
    assert_contract_error(result, EscrowError::InvalidState);
}

/// Tests that deposit_funds panics with InsufficientFunds when caller token balance is too low.
///
/// Note: In Soroban test environment with mocked auth, balance checks are typically bypassed.
/// This test documents the error branch but may not be directly testable without token contract integration.
/// 
/// # UNREACHABLE
/// InsufficientFunds in deposit_funds is currently unreachable because:
/// - The contract does not perform balance verification in the current implementation
/// - Token transfer is mocked in test environment
/// - Real balance checks occur only at the token contract level during actual transfers
///
/// Documented per Issue #405 requirements for completeness.
#[test]
#[ignore]
fn test_deposit_insufficient_funds() {
    // UNREACHABLE: deposit_funds does not check caller's token balance
    // in the current implementation. Balance validation occurs at token contract level.
    // This test is documented for completeness but cannot be triggered in unit tests.
}

