//! Deterministic TTL / expiration policy for transient and persistent storage.

use crate::{DataKey, Error, Milestone};
use soroban_sdk::{Env, IntoVal, TryFromVal, Val, Vec};

pub const LEDGERS_PER_DAY: u32 = 17_280;
pub const PENDING_APPROVAL_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 7;
pub const PENDING_APPROVAL_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY;
pub const MIN_APPROVAL_TTL: u32 = 17_280;
pub const ADMIN_ROTATION_MIN_DELAY_LEDGERS: u32 = LEDGERS_PER_DAY * 2;
pub const PENDING_MIGRATION_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 21;
pub const PENDING_MIGRATION_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 3;
pub const PERSISTENT_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 30;
pub const PERSISTENT_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 7;

pub fn compute_expiry(env: &Env, ttl_ledgers: u32) -> u32 {
    env.ledger().sequence().saturating_add(ttl_ledgers)
}

pub fn store_with_ttl<K, V>(env: &Env, key: &K, value: &V, ttl_ledgers: u32)
where
    K: IntoVal<Env, Val>,
    V: IntoVal<Env, Val>,
{
    let storage = env.storage().temporary();
    storage.set(key, value);
    storage.extend_ttl(key, ttl_ledgers, ttl_ledgers);
}

pub fn read_if_live<K, V>(env: &Env, key: &K) -> Option<V>
where
    K: IntoVal<Env, Val>,
    V: TryFromVal<Env, Val>,
{
    env.storage().temporary().get(key)
}

pub fn extend_if_below_threshold<K>(env: &Env, key: &K, threshold: u32, extend_to: u32) -> bool
where
    K: IntoVal<Env, Val>,
{
    let storage = env.storage().temporary();
    if !storage.has(key) {
        return false;
    }
    storage.extend_ttl(key, threshold, extend_to);
    true
}

pub fn remove_transient<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    env.storage().temporary().remove(key);
}

pub fn has_transient<K>(env: &Env, key: &K) -> bool
where
    K: IntoVal<Env, Val>,
{
    env.storage().temporary().has(key)
}

/// Loads the persistent milestone vector for `contract_id` and bumps its
/// persistent TTL.
///
/// This is the **single canonical read path** for milestone vectors across
/// the escrow contract (see issue #701). Centralising access here
/// normalises three concerns:
///
/// 1. **Composite key** — built exactly once via [`milestone_storage_key`].
///    No inline `Symbol::new(&env, "milestones")` literals remain.
/// 2. **Missing-entry error** — always `Error::ContractNotFound`. Open-coded
///    sites previously mixed `.unwrap()` / `panic_with_error` /
///    `ok_or(ContractNotFound)` and confused off-chain tooling.
/// 3. **TTL bump** — always uses `PERSISTENT_BUMP_THRESHOLD` /
///    `PERSISTENT_TTL_LEDGERS` so the entry cannot be silently archived
///    between two reads in the same call frame.
///
/// # Arguments
/// * `env`  - The contract environment (must be inside an `as_contract`
///            scope when invoked from a `#[test]` harness).
/// * `contract_id` - The `u32` identifier previously returned by
///                    [`crate::create_contract`].
///
/// # Returns
/// The `Vec<Milestone>` currently persisted under the composite key
/// `(DataKey::Contract(contract_id), Symbol("milestones"))`.
///
/// # Panics
/// Panics with [`Error::ContractNotFound`] when the milestone vector is
/// absent or has been archived by the host. Call sites that need
/// different failure semantics must use [`try_load_milestones`] instead.
///
/// # Side effects
/// Extends the milestone entry's persistent TTL via
/// [`extend_milestone_ttl`].
///
/// # See also
/// - [`try_load_milestones`] — non-panicking variant.
/// - [`store_milestones`] — symmetric write path.
pub fn load_milestones(env: &Env, contract_id: u32) -> Vec<Milestone> {
    let key = MilestonesKey::new(contract_id);
    let milestones: Vec<Milestone> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(crate::EscrowError::ContractNotFound));
    extend_milestone_ttl(env, contract_id);
    milestones
}

/// Non-panicking counterpart to [`load_milestones`].
///
/// Returns `Some(Vec<Milestone>)` when the milestone vector is present
/// (and bumps its persistent TTL), or `None` when it is absent. Use this
/// in read-only paths where a missing milestone vector is a non-error
/// outcome (e.g. `is_milestone_overdue` for an arbitrary caller-supplied
/// `contract_id`).
///
/// # Arguments
/// * `env`  - The contract environment.
/// * `contract_id` - The `u32` identifier under which a milestone vector
///                    may or may not exist.
///
/// # Returns
/// * `Some(Vec<Milestone>)` — the persisted vector, with TTL bumped.
/// * `None` — no milestone vector is persisted for `contract_id`. No TTL
///            bump is performed on this branch (there is nothing to bump).
pub fn try_load_milestones(env: &Env, contract_id: u32) -> Option<Vec<Milestone>> {
    let key = milestone_storage_key(env, contract_id);
    let milestones: Option<Vec<Milestone>> = env.storage().persistent().get(&key);
    if milestones.is_some() {
        extend_milestone_ttl(env, contract_id);
    }
    milestones
}

/// Persists `milestones` for `contract_id` under the canonical composite
/// key and bumps the persistent TTL.
///
/// This is the **single canonical write path** for milestone vectors.
/// Every entrypoint that mutates milestone state (e.g. `release_milestone`,
/// `refund_unreleased_milestones`, `submit_work_evidence`, approval flows,
/// creation) must funnel through this helper so the lives of three
/// concerns stay in lock-step:
///
/// 1. **Composite key** — built once via [`milestone_storage_key`].
/// 2. **Atomic write + TTL bump** — the TTL is bumped in the same
///    logical step as the write, so a freshly-stored vector cannot be
///    archived in the same ledger window.
/// 3. **Bump parameters** — `PERSISTENT_BUMP_THRESHOLD` /
///    `PERSISTENT_TTL_LEDGERS`, identical to the read path's bump.
///
/// # Arguments
/// * `env`         - The contract environment.
/// * `contract_id` - The `u32` identifier previously allocated by
///                    [`crate::create_contract`].
/// * `milestones`  - The new vector to persist.
///
/// # See also
/// - [`load_milestones`] — the symmetric read path.
pub fn store_milestones(env: &Env, contract_id: u32, milestones: &Vec<Milestone>) {
    let key = MilestonesKey::new(contract_id);
    env.storage().persistent().set(&key, milestones);
    extend_milestone_ttl(env, contract_id);
}

pub(crate) fn milestone_storage_key(_env: &Env, contract_id: u32) -> DataKey {
    DataKey::Milestones(contract_id)
}

pub fn extend_next_contract_id_ttl(env: &Env) {
    if env.storage().persistent().has(&DataKey::NextContractId) {
        env.storage().persistent().extend_ttl(
            &DataKey::NextContractId,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_TTL_LEDGERS,
        );
    }
}

pub fn extend_contract_ttl(env: &Env, contract_id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::Contract(contract_id),
        PERSISTENT_BUMP_THRESHOLD,
        PERSISTENT_TTL_LEDGERS,
    );
}

pub fn extend_milestone_ttl(env: &Env, contract_id: u32) {
    env.storage().persistent().extend_ttl(
        &MilestonesKey::new(contract_id),
        PERSISTENT_BUMP_THRESHOLD,
        PERSISTENT_TTL_LEDGERS,
    );
}

pub fn extend_contract_and_milestones_ttl(env: &Env, contract_id: u32) {
    extend_contract_ttl(env, contract_id);
    extend_milestone_ttl(env, contract_id);
}

pub fn extend_participant_contract_index_ttl(env: &Env, key: &crate::DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS);
}

/// Extend TTL of the dispute record entry for `contract_id`.
pub fn extend_dispute_ttl(env: &Env, contract_id: u32) {
    let key = DataKey::Dispute(contract_id);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_TTL_LEDGERS,
        );
    }
}
