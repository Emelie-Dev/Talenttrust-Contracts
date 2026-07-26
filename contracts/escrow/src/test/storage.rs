use super::{
    assert_contract_error, complete_contract, create_contract, default_milestones,
    generated_participants, register_client, total_milestone_amount, MILESTONE_ONE, MILESTONE_THREE,
    MILESTONE_TWO,
};
use crate::{
    ContractStatus, DataKey, EscrowError, Milestone, MilestonesKey, ReadinessChecklist,
    ReleaseAuthorization,
};
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol, Vec as SorobanVec};

// ─── Initialized / Admin ──────────────────────────────────────────────────────

#[test]
fn initialized_written_on_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);

    assert!(client.initialize(&admin));

    env.as_contract(&client.address, || {
        let v: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Initialized)
            .unwrap();
        assert!(v);
    });
}

#[test]
fn admin_written_on_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    env.as_contract(&client.address, || {
        let stored: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        assert_eq!(stored, admin);
    });
}

#[test]
fn double_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);
    assert_contract_error(
        client.try_initialize(&admin),
        EscrowError::AlreadyInitialized,
    );
}

// ─── Paused ───────────────────────────────────────────────────────────────────

#[test]
fn paused_written_by_pause_and_cleared_by_unpause() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    client.pause();
    env.as_contract(&client.address, || {
        let v: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        assert!(v);
    });

    client.unpause();
    env.as_contract(&client.address, || {
        let v: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        assert!(!v);
    });
}

#[test]
fn paused_blocks_create_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    client.pause();

    let (c, f) = generated_participants(&env);
    assert_contract_error(
        client.try_create_contract(
            &c,
            &f,
            &None,
            &default_milestones(&env),
            &ReleaseAuthorization::ClientOnly,
        ),
        EscrowError::ContractPaused,
    );
}

#[test]
fn paused_blocks_deposit_funds() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.pause();

    assert_contract_error(
        client.try_deposit_funds(&id, &client_addr, &total_milestone_amount()),
        EscrowError::ContractPaused,
    );
}

#[test]
fn paused_blocks_release_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.deposit_funds(&id, &client_addr, &total_milestone_amount());
    client.pause();

    assert_contract_error(
        client.try_release_milestone(&id, &client_addr, &0),
        EscrowError::ContractPaused,
    );
}

#[test]
fn paused_blocks_cancel_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.pause();

    assert_contract_error(
        client.try_cancel_contract(&id, &client_addr),
        EscrowError::ContractPaused,
    );
}

#[test]
fn read_only_queries_not_blocked_by_pause() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    let (_, _, id) = create_contract(&env, &client);
    client.pause();

    let record = client.get_contract(&id);
    assert_eq!(record.status, ContractStatus::Created);
    assert!(client.is_paused());
}

// ─── Emergency ────────────────────────────────────────────────────────────────

#[test]
fn emergency_written_by_activate_and_cleared_by_resolve() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    client.activate_emergency_pause();
    env.as_contract(&client.address, || {
        let v: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Emergency)
            .unwrap_or(false);
        assert!(v);
    });

    client.resolve_emergency();
    env.as_contract(&client.address, || {
        let v: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Emergency)
            .unwrap_or(false);
        assert!(!v);
    });
}

#[test]
fn unpause_blocked_while_emergency_active() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    client.activate_emergency_pause();
    assert_contract_error(client.try_unpause(), EscrowError::EmergencyActive);
}

// ─── Contract / NextContractId ────────────────────────────────────────────────

#[test]
fn contract_written_on_create_and_readable() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (c, f) = generated_participants(&env);

    let id = client.create_contract(
        &c,
        &f,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );

    let record = client.get_contract(&id);
    assert_eq!(record.client, c);
    assert_eq!(record.freelancer, f);
    assert_eq!(record.status, ContractStatus::Created);
}

#[test]
fn next_contract_id_increments_per_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (_, _, id1) = create_contract(&env, &client);
    let (_, _, id2) = create_contract(&env, &client);
    assert_eq!(id2, id1 + 1);
}

#[test]
fn get_contract_fails_for_unknown_id() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    assert_contract_error(
        client.try_get_contract(&9999),
        EscrowError::ContractNotFound,
    );
}

// ─── Milestone released flag (milestone vector) ───────────────────────────────

/// `release_milestone` sets `ms.released = true` in the persisted milestone
/// vector. There is no separate `DataKey::MilestoneReleased` storage key; the
/// vector is the single source of truth for released state.
#[test]
fn milestone_released_flag_set_in_vector_on_release() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.deposit_funds(&id, &client_addr, &total_milestone_amount());
    client.approve_milestone_release(&id, &client_addr, &0);
    client.release_milestone(&id, &client_addr, &0);

    let milestones = client.get_milestones(&id);
    assert!(milestones.get(0).unwrap().released, "index 0 must be released");
    assert!(!milestones.get(1).unwrap().released, "index 1 must not be released");
    assert!(!milestones.get(2).unwrap().released, "index 2 must not be released");
}

#[test]
fn double_release_same_milestone_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.deposit_funds(&id, &client_addr, &total_milestone_amount());
    client.release_milestone(&id, &client_addr, &0);

    assert_contract_error(
        client.try_release_milestone(&id, &client_addr, &0),
        EscrowError::AlreadyReleased,
    );
}

#[test]
fn release_out_of_bounds_milestone_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.deposit_funds(&id, &client_addr, &total_milestone_amount());

    assert_contract_error(
        client.try_release_milestone(&id, &client_addr, &99),
        EscrowError::InvalidMilestone,
    );
}

// ─── ReputationIssued / Reputation / PendingReputationCredits ─────────────────

#[test]
fn reputation_issued_written_and_reputation_updated() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (c, f, id) = complete_contract(&env, &client);
    client.issue_reputation(&id, &c, &f, &5);

    env.as_contract(&client.address, || {
        let issued: bool = env
            .storage()
            .persistent()
            .get(&DataKey::ReputationIssued(id))
            .unwrap_or(false);
        assert!(issued);
    });

    let rep = client.get_reputation(&f).unwrap();
    assert_eq!(rep.completed_contracts, 1);
    assert_eq!(rep.total_rating, 5);
    assert_eq!(rep.last_rating, 5);
}

#[test]
fn double_issue_reputation_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (c, f, id) = complete_contract(&env, &client);
    client.issue_reputation(&id, &c, &f, &4);

    assert_contract_error(
        client.try_issue_reputation(&id, &c, &f, &4),
        EscrowError::ReputationAlreadyIssued,
    );
}

#[test]
fn pending_reputation_credits_incremented_on_completion() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (_, f, _) = complete_contract(&env, &client);
    assert_eq!(client.get_pending_reputation_credits(&f), 1);
}

#[test]
fn pending_reputation_credits_decremented_on_issue() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (c, f, id) = complete_contract(&env, &client);
    assert_eq!(client.get_pending_reputation_credits(&f), 1);

    client.issue_reputation(&id, &c, &f, &3);
    assert_eq!(client.get_pending_reputation_credits(&f), 0);
}

#[test]
fn reputation_not_issuable_before_completion() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (c, f) = generated_participants(&env);
    let id = client.create_contract(
        &c,
        &f,
        &None,
        &default_milestones(&env),
        &ReleaseAuthorization::ClientOnly,
    );

    assert_contract_error(
        client.try_issue_reputation(&id, &c, &f, &5),
        EscrowError::NotCompleted,
    );
}

#[test]
fn reputation_requires_client_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (c, f, id) = complete_contract(&env, &client);
    let stranger = Address::generate(&env);

    assert_contract_error(
        client.try_issue_reputation(&id, &stranger, &f, &5),
        EscrowError::UnauthorizedRole,
    );
}

// ─── ReadinessChecklist ───────────────────────────────────────────────────────

#[test]
fn readiness_checklist_initialized_flag_set_by_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    env.as_contract(&client.address, || {
        let checklist: ReadinessChecklist = env
            .storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap();
        assert!(checklist.initialized);
        assert!(!checklist.governed_params_set);
    });
}

#[test]
fn readiness_checklist_emergency_flag_set_by_activate() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);

    client.activate_emergency_pause();

    env.as_contract(&client.address, || {
        let checklist: ReadinessChecklist = env
            .storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap();
        assert!(checklist.emergency_controls_enabled);
    });
}

// ─── Accounting invariant ─────────────────────────────────────────────────────

#[test]
fn released_amount_tracks_milestone_amounts() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr, _, id) = create_contract(&env, &client);
    client.deposit_funds(&id, &client_addr, &total_milestone_amount());

    client.release_milestone(&id, &client_addr, &0);
    let r = client.get_contract(&id);
    assert_eq!(r.released_amount, MILESTONE_ONE);

    client.release_milestone(&id, &client_addr, &1);
    let r = client.get_contract(&id);
    assert_eq!(r.released_amount, MILESTONE_ONE + MILESTONE_TWO);

    client.release_milestone(&id, &client_addr, &2);
    let r = client.get_contract(&id);
    assert_eq!(r.released_amount, total_milestone_amount());
    assert_eq!(r.status, ContractStatus::Completed);
}

// ─── get_milestone single-index reader (issue #649) ───────────────────────────

#[test]
fn get_milestone_index_zero_returns_first_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    // Default contract has three milestones: ONE, TWO, THREE.
    let (_client_addr, _, id) = create_contract(&env, &client);

    let m = client
        .get_milestone(&id, &0u32)
        .expect("index 0 is in bounds");
    assert_eq!(m.amount, MILESTONE_ONE);
    // It must match the entry returned by the full-vector reader.
    assert_eq!(m, client.get_milestones(&id).get(0).unwrap());
}

#[test]
fn get_milestone_last_valid_index_returns_last_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _, id) = create_contract(&env, &client);

    let milestones = client.get_milestones(&id);
    let last = milestones.len() - 1;
    let m = client
        .get_milestone(&id, &last)
        .expect("last index is in bounds");
    assert_eq!(m.amount, MILESTONE_THREE);
    assert_eq!(m, milestones.get(last).unwrap());
}

#[test]
fn get_milestone_out_of_bounds_returns_none() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _, id) = create_contract(&env, &client);

    let len = client.get_milestones(&id).len();
    // One past the last valid index must return None, not panic.
    assert!(client.get_milestone(&id, &len).is_none());
    assert!(client.get_milestone(&id, &(len + 5)).is_none());
}

#[test]
fn get_milestone_unknown_contract_panics_contract_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    // No contract has been created; id 999 was never allocated.
    assert_contract_error(
        client.try_get_milestone(&999u32, &0u32),
        EscrowError::ContractNotFound,
    );
}

#[test]
fn deposit_exceeding_total_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let (client_addr, _, id) = create_contract(&env, &client);
    assert_contract_error(
        client.try_deposit_funds(&id, &client_addr, &(total_milestone_amount() + 1)),
        EscrowError::ExactDepositRequired,
    );
}

// ─── MilestonesKey typed storage key (issue #938) ──────────────────────────
//
// These tests pin the byte-compatible contract of [`MilestonesKey`]:
// reads/writes performed through the typed key are interchangeable with reads
// and writes performed through the legacy `(DataKey::Contract(id),
// Symbol::new(&env, "milestones"))` tuple. The on-disk storage bytes must
// match so contracts persisted before the refactor remain reachable.

/// Build a tiny milestone vector with deterministic amount / funded_amount
/// values so round-trip equality assertions are robust.
fn three_milestones(env: &Env) -> SorobanVec<Milestone> {
    SorobanVec::from_array(
        env,
        [
            Milestone {
                amount: 100,
                funded_amount: 0,
                released: false,
                refunded: false,
                work_evidence: None,
                refunded_amount: 0,
                deadline: None,
            },
            Milestone {
                amount: 200,
                funded_amount: 0,
                released: false,
                refunded: false,
                work_evidence: None,
                refunded_amount: 0,
                deadline: None,
            },
            Milestone {
                amount: 300,
                funded_amount: 0,
                released: false,
                refunded: false,
                work_evidence: None,
                refunded_amount: 0,
                deadline: None,
            },
        ],
    )
}

#[test]
fn milestones_key_into_val_matches_legacy_tuple_into_val() {
    let env = Env::default();
    let contract_id: u32 = 17;

    let key_val: soroban_sdk::Val = MilestonesKey::new(contract_id).into_val(&env);
    let tuple_val: soroban_sdk::Val = (
        DataKey::Contract(contract_id),
        Symbol::new(&env, crate::MILESTONES_STORAGE_SYMBOL),
    )
        .into_val(&env);

    // Forcing both through the same SCVal conversion path proves the two
    // keys collide on the host's storage hash map. Any drift here would
    // silently brick contracts that were written before the refactor.
    let key_scval: soroban_sdk::xdr::ScVal = (&key_val).try_into_val(&env).unwrap();
    let tuple_scval: soroban_sdk::xdr::ScVal = (&tuple_val).try_into_val(&env).unwrap();
    assert_eq!(key_scval, tuple_scval);
}

#[test]
fn milestones_key_try_from_val_round_trips_legacy_tuple_val() {
    let env = Env::default();
    let contract_id: u32 = 42;

    let tuple_val: soroban_sdk::Val = (
        DataKey::Contract(contract_id),
        Symbol::new(&env, crate::MILESTONES_STORAGE_SYMBOL),
    )
        .into_val(&env);
    let key: MilestonesKey = (&tuple_val).try_into_val(&env).unwrap();
    assert_eq!(key, MilestonesKey::new(contract_id));
    assert_eq!(key.contract_id(), contract_id);
}

#[test]
fn milestones_key_try_from_val_rejects_wrong_first_component() {
    let env = Env::default();
    // A mis-typed first component (e.g. DataKey::Admin) must NOT resolve to a
    // milestones key, even though soroban-sdk can technically decode a
    // tuple-shaped Val. This is the protective invariant.
    let bogus_val: soroban_sdk::Val = (
        DataKey::Admin,
        Symbol::new(&env, crate::MILESTONES_STORAGE_SYMBOL),
    )
        .into_val(&env);
    let result: Result<MilestonesKey, _> = (&bogus_val).try_into_val(&env);
    assert!(
        result.is_err(),
        "MilestonesKey::try_from_val must reject non-Contract first components; got {:?}",
        result.ok()
    );
}

#[test]
fn milestones_key_try_from_val_rejects_wrong_symbol() {
    let env = Env::default();
    // A wrong second component (different symbol) must NOT resolve.
    let bogus_val: soroban_sdk::Val =
        (DataKey::Contract(7u32), Symbol::new(&env, "not-milestones")).into_val(&env);
    let result: Result<MilestonesKey, _> = (&bogus_val).try_into_val(&env);
    assert!(
        result.is_err(),
        "MilestonesKey::try_from_val must reject foreign symbols; got {:?}",
        result.ok()
    );
}

#[test]
fn write_with_legacy_tuple_read_with_milestones_key() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let escrow_addr = client.address.clone();
    let contract_id: u32 = 1;

    env.as_contract(&escrow_addr, || {
        let store = env.storage().persistent();
        // Write via the legacy tuple (this is what write_sites pre-#938 did).
        store.set(
            &(DataKey::Contract(contract_id), Symbol::new(&env, "milestones")),
            &three_milestones(&env),
        );
    });

    env.as_contract(&escrow_addr, || {
        // Read via the typed key. Must return the same vector.
        let read_typed: SorobanVec<Milestone> = env
            .storage()
            .persistent()
            .get(&MilestonesKey::new(contract_id))
            .expect("milestones should be readable via typed key");
        let read_legacy: SorobanVec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), Symbol::new(&env, "milestones")))
            .expect("milestones should be readable via legacy tuple");
        assert_eq!(read_typed, read_legacy);
        assert_eq!(read_typed.len(), 3);
        assert_eq!(read_typed.get(0).unwrap().amount, 100);
        assert_eq!(read_typed.get(2).unwrap().amount, 300);
    });
}

#[test]
fn write_with_milestones_key_read_with_legacy_tuple() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let escrow_addr = client.address.clone();
    let contract_id: u32 = 1;

    env.as_contract(&escrow_addr, || {
        // Write via the typed key (this is what write_sites post-#938 do).
        env.storage()
            .persistent()
            .set(&MilestonesKey::new(contract_id), &three_milestones(&env));
    });

    env.as_contract(&escrow_addr, || {
        // Read via the legacy tuple. Must return the same vector.
        let via_legacy: SorobanVec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), Symbol::new(&env, "milestones")))
            .expect("legacy tuple read must succeed after typed-key write");
        assert_eq!(via_legacy.len(), 3);
        assert_eq!(via_legacy.get(1).unwrap().amount, 200);

        // Also confirm `has_milestones()` (the typed `has`) returns true.
        assert!(crate::ttl::has_milestones(&env, contract_id));
    });
}

#[test]
fn milestones_key_has_returns_false_for_absent_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    env.as_contract(&client.address, || {
        assert!(
            !crate::ttl::has_milestones(&env, 9999),
            "absent milestones entry must report false (typed `has`)"
        );
        assert!(
            !env.storage()
                .persistent()
                .has(&MilestonesKey::new(9999)),
            "absent milestones entry must report false (direct `has` call)"
        );
    });
}

#[test]
fn milestones_key_has_returns_true_after_store() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&MilestonesKey::new(99), &three_milestones(&env));
        assert!(crate::ttl::has_milestones(&env, 99));
        assert!(
            env.storage()
                .persistent()
                .has(&MilestonesKey::new(99)),
            "present milestones entry must report true"
        );
        // A neighbouring id was never written — must remain false.
        assert!(!crate::ttl::has_milestones(&env, 100));
    });
}

#[test]
fn milestones_key_extend_ttl_succeeds_via_typed_key() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    env.ledger().with_mut(|li| {
        li.max_entry_ttl = 10_000;
        li.min_persistent_entry_ttl = 10_000;
    });

    let contract_id: u32 = 5;
    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&MilestonesKey::new(contract_id), &three_milestones(&env));
    });

    env.as_contract(&client.address, || {
        // Generic extend_ttl through the typed key. Must not panic; this
        // exercises every call site that uses
        // `env.storage().persistent().extend_ttl(&MilestonesKey::new(id), ...)`.
        env.storage().persistent().extend_ttl(
            &MilestonesKey::new(contract_id),
            100,
            5_000,
        );
        let stored: SorobanVec<Milestone> = env
            .storage()
            .persistent()
            .get(&MilestonesKey::new(contract_id))
            .expect("value should survive extend_ttl");
        assert_eq!(stored.len(), 3);
    });
}

#[test]
fn has_milestones_returns_false_after_remove() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&MilestonesKey::new(3), &three_milestones(&env));
        assert!(crate::ttl::has_milestones(&env, 3));
        env.storage()
            .persistent()
            .remove(&MilestonesKey::new(3));
        assert!(
            !crate::ttl::has_milestones(&env, 3),
            "removed milestones entry must report false"
        );
        assert!(
            env.storage()
                .persistent()
                .get::<_, SorobanVec<Milestone>>(&MilestonesKey::new(3))
                .is_none(),
            "removed milestones entry must return None on read"
        );
    });
}

#[test]
fn milestone_storage_key_helper_returns_milestones_key() {
    let env = Env::default();
    let k = crate::ttl::milestone_storage_key(&env, 8);
    assert_eq!(k, MilestonesKey::new(8));
    assert_eq!(k.contract_id(), 8);
}
