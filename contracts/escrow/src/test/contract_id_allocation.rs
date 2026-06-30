//! Contract-id allocation invariant tests.
//!
//! # Invariants verified
//! 1. Ids are allocated sequentially starting from 1 with no gaps.
//! 2. Each id is unique — no two live contracts share an id.
//! 3. `NextContractId` is advanced by exactly 1 after every successful create.
//! 4. A `ContractIdOverflow` error is returned when the counter is at `u32::MAX`.
//! 5. A `ContractIdCollision` error is returned when the target slot is occupied.
//! 6. Neither overflow nor collision mutates `NextContractId`.

use super::{default_milestones, generated_participants, register_client};
use crate::{DataKey, Error, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, Address, Env};

// -----------------------------------------------------------------------
// Helper
// -----------------------------------------------------------------------

fn assert_error<T: core::fmt::Debug>(
    result: Result<
        Result<T, soroban_sdk::ConversionError>,
        Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
    >,
    expected: Error,
) {
    match result {
        Err(Ok(e)) => {
            let expected_err: soroban_sdk::Error = expected.into();
            assert_eq!(e, expected_err);
        }
        other => panic!("expected {:?}, got {:?}", expected, other),
    }
}

/// Read the persisted NextContractId counter directly from storage.
fn read_next_id(env: &Env, escrow_addr: &soroban_sdk::Address) -> u32 {
    env.as_contract(escrow_addr, || {
        env.storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1)
    })
}

// -----------------------------------------------------------------------
// Sequential / gap-free allocation
// -----------------------------------------------------------------------

/// The first contract ever created must receive id = 1 (the default seed).
#[test]
fn first_contract_id_is_one() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client, freelancer, _) = generated_participants(&env);
    let milestones = default_milestones(&env);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq!(id, 1, "first allocated id must be 1");
}

/// Sequential creates must return ids 1, 2, 3, … with no gaps.
#[test]
fn ids_are_sequential_and_gap_free() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let milestones = default_milestones(&env);
    let count: u32 = 10;

    let mut ids: soroban_sdk::Vec<u32> = soroban_sdk::Vec::new(&env);
    for _ in 0..count {
        let (client, freelancer, _) = generated_participants(&env);
        let id = escrow.create_contract(
            &client,
            &freelancer,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        );
        ids.push_back(id);
    }

    for (i, id) in ids.iter().enumerate() {
        assert_eq!(id, (i as u32) + 1, "id at position {i} should be {}", i + 1);
    }
}

/// After N creates the stored counter equals N + 1 (ready for the next create).
#[test]
fn counter_advances_exactly_one_per_create() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let milestones = default_milestones(&env);
    let count: u32 = 5;

    for i in 0..count {
        let (client, freelancer, _) = generated_participants(&env);
        escrow.create_contract(
            &client,
            &freelancer,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        );
        let next = read_next_id(&env, &escrow.address);
        assert_eq!(next, i + 2, "counter after {}", i + 1);
    }
}

/// All allocated ids must be unique across many sequential creates.
#[test]
fn all_ids_are_unique() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let milestones = default_milestones(&env);
    let count: u32 = 20;

    let mut seen: soroban_sdk::Vec<u32> = soroban_sdk::Vec::new(&env);
    for _ in 0..count {
        let (client, freelancer, _) = generated_participants(&env);
        let id = escrow.create_contract(
            &client,
            &freelancer,
            &None,
            &milestones,
            &ReleaseAuthorization::ClientOnly,
        );
        // Verify no duplicate
        for existing in seen.iter() {
            assert_ne!(existing, id, "duplicate id {id} detected");
        }
        seen.push_back(id);
    }
    assert_eq!(seen.len(), count);
}

// -----------------------------------------------------------------------
// Overflow protection
// -----------------------------------------------------------------------

/// When `NextContractId` is `u32::MAX` the counter cannot be advanced and
/// `ContractIdOverflow` must be returned.  The counter must remain unchanged.
#[test]
fn next_contract_id_overflow_at_u32_max() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, _) = generated_participants(&env);
    let milestones = default_milestones(&env);

    env.as_contract(&escrow.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &u32::MAX);
    });

    let result = escrow.try_create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_error(result, Error::ContractIdOverflow);

    // Counter must not have moved.
    let after = read_next_id(&env, &escrow.address);
    assert_eq!(after, u32::MAX, "counter must not change on overflow");
}

/// Overflow at `u32::MAX - 1` does not fire; at `u32::MAX` it does.
#[test]
fn overflow_fires_only_at_u32_max_not_before() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client, freelancer, _) = generated_participants(&env);
    let milestones = default_milestones(&env);

    // Place counter at u32::MAX - 1; the create should succeed.
    env.as_contract(&escrow.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &(u32::MAX - 1));
    });

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_eq!(id, u32::MAX - 1);

    // Counter is now u32::MAX; next create must overflow.
    let after = read_next_id(&env, &escrow.address);
    assert_eq!(after, u32::MAX);

    let (c2, f2, _) = generated_participants(&env);
    let result = escrow.try_create_contract(
        &c2,
        &f2,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_error(result, Error::ContractIdOverflow);
}

// -----------------------------------------------------------------------
// Collision protection
// -----------------------------------------------------------------------

/// `ContractIdCollision` fires when the target slot is already occupied and
/// `NextContractId` must not change.
#[test]
fn next_contract_id_rejects_occupied_slot() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client_addr, freelancer_addr, _) = generated_participants(&env);
    let milestones = default_milestones(&env);

    let existing_id = escrow.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    // Wind the counter back so the next allocation targets the occupied slot.
    env.as_contract(&escrow.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &existing_id);
    });

    let intruder = Address::generate(&env);
    let result = escrow.try_create_contract(
        &intruder,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_error(result, Error::ContractIdCollision);

    // Counter must not have advanced past the collision point.
    let after = read_next_id(&env, &escrow.address);
    assert_eq!(after, existing_id, "counter must not advance on collision");
}

/// A collision does not corrupt subsequent creates once the counter is fixed.
#[test]
fn allocation_resumes_correctly_after_counter_is_repaired() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client, freelancer, _) = generated_participants(&env);
    let milestones = default_milestones(&env);

    // Create id=1, then wind counter back to 1 (simulating corruption).
    escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    env.as_contract(&escrow.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &1u32);
    });

    // Attempted create collides at id=1.
    let (c2, f2, _) = generated_participants(&env);
    let collision = escrow.try_create_contract(
        &c2,
        &f2,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_error(collision, Error::ContractIdCollision);

    // Repair: advance counter to 2 (what it should have been).
    env.as_contract(&escrow.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &2u32);
    });

    // Next create must succeed at id=2.
    let (c3, f3, _) = generated_participants(&env);
    let id = escrow.create_contract(
        &c3,
        &f3,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert_eq!(id, 2);
}

// -----------------------------------------------------------------------
// Single-call allocation — no intermediate state exposed
// -----------------------------------------------------------------------

/// Only one contract is stored per `create_contract` call.
/// This guards against the old double-call bug leaving phantom storage entries.
#[test]
fn single_create_stores_exactly_one_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let (client, freelancer, _) = generated_participants(&env);
    let milestones = default_milestones(&env);

    let id = escrow.create_contract(
        &client,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    // Exactly the returned id should be in storage.
    env.as_contract(&escrow.address, || {
        let exists: bool = env
            .storage()
            .persistent()
            .has(&DataKey::Contract(id));
        assert!(exists, "contract {id} should be in storage");

        // No other ids should exist (0, 2, … are all absent after first create).
        let phantom0: bool = env
            .storage()
            .persistent()
            .has(&DataKey::Contract(0));
        assert!(!phantom0, "phantom contract at id 0 must not exist");

        let phantom2: bool = env
            .storage()
            .persistent()
            .has(&DataKey::Contract(id + 1));
        assert!(!phantom2, "phantom contract at id+1 must not exist");
    });
}
