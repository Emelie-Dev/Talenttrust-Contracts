//! Deterministic TTL / expiration policy for transient and persistent storage.
//!
//! This module defines all time‑to‑live (TTL) constants used by the escrow contract and provides
//! helper utilities for storing, reading and extending entries. The constants are expressed in
//! **ledger counts** – on Stellar mainnet a ledger is ~5 seconds. For readability we also expose the
//! equivalent number of days.
//!
//! | Constant                              | Ledger count | Days (≈) | Governs
//! |--------------------------------------|--------------|----------|------------------------------------------------------------
//! | `LEDGERS_PER_DAY`                    | 17_280       | 1        | conversion factor
//! | `PENDING_APPROVAL_TTL_LEDGERS`       | 120_960      | 7        | transient approvals stored in `temporary()`
//! | `PENDING_MIGRATION_TTL_LEDGERS`      | 362_880      | 21       | transient migration requests in `temporary()`
//! | `PERSISTENT_TTL_LEDGERS`             | 518_400      | 30       | persistent contract data stored in `persistent()`
//! | `PENDING_APPROVAL_BUMP_THRESHOLD`    | 17_280       | 1        | when a read occurs within this many ledgers of expiry, its TTL is bumped
//! | `PENDING_MIGRATION_BUMP_THRESHOLD`   | 51_840       | 3        | same, but for migrations
//! | `PERSISTENT_BUMP_THRESHOLD`          | 120_960      | 7        | bump threshold for persistent entries
//!
//! **Bump‑on‑read strategy** – The `extend_if_below_threshold` helper is used by entry‑point
//! implementations to extend the TTL of a transient entry when it is accessed and the remaining
//! lifetime falls below the corresponding *bump threshold*. This ensures that active approvals or
//! migrations survive a series of reads without being evicted, while still allowing them to expire
//! if they become stale.
//!
//! **Eviction risk** – If a contract (or its milestone vector) is never accessed for more than
//! `PERSISTENT_TTL_LEDGERS` (30 days) the Soroban host will evict the persistent storage entry. The
//! contract then becomes inaccessible; any subsequent reads will return `None`. This is a deliberate
//! safety measure – stale contracts are archived automatically.
//!
//! **`read_if_live` semantics** – The `read_if_live` helper reads from `temporary()` storage and
//! returns `None` for two distinct cases:
//!   1. The key was never set ("absent").
//!   2. The key was set but its TTL has expired and the entry was evicted.
//! This "fail‑closed" behaviour is important for approvals and migrations: a missing entry is
//! interpreted as not approved/not migrated, preventing any stale permission from being honored.
//!
//! Storage ownership: this module owns TTL policy and helper access patterns,
//! not business records. It extends caller-provided keys, with first-class
//! helpers for `DataKey::Contract(contract_id)`, the paired milestone vector
//! key `(DataKey::Contract(contract_id), "milestones")`, `NextContractId`,
//! participant index keys, pending approvals, and pending migrations.
//!
use crate::{DataKey, Error, Milestone};
use soroban_sdk::{Env, IntoVal, Symbol, TryFromVal, Val, Vec};

pub const LEDGERS_PER_DAY: u32 = 17_280;

pub const PENDING_APPROVAL_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 7;
pub const PENDING_APPROVAL_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY;
pub const MIN_APPROVAL_TTL: u32 = 17_280;

/// Minimum ledgers that must elapse between proposing and finalising a
/// treasury / admin rotation. At ~5 s per ledger this is roughly 2 days,
/// giving stakeholders time to react to an unexpected proposal.
pub const ADMIN_ROTATION_MIN_DELAY_LEDGERS: u32 = LEDGERS_PER_DAY * 2;

pub const PENDING_MIGRATION_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 21;
pub const PENDING_MIGRATION_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 3;

/// Persistent storage TTL: extend to 30 days, renew when below 7 days.
pub const PERSISTENT_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 30;
pub const PERSISTENT_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 7;

#[allow(dead_code)]
pub fn compute_expiry(env: &Env, ttl_ledgers: u32) -> u32 {
    env.ledger().sequence().saturating_add(ttl_ledgers)
}

#[allow(dead_code)]
pub fn store_with_ttl<K, V>(env: &Env, key: &K, value: &V, ttl_ledgers: u32)
where
    K: IntoVal<Env, Val>,
    V: IntoVal<Env, Val>,
{
    let storage = env.storage().temporary();
    storage.set(key, value);
    storage.extend_ttl(key, ttl_ledgers, ttl_ledgers);
}

#[allow(dead_code)]
pub fn read_if_live<K, V>(env: &Env, key: &K) -> Option<V>
where
    K: IntoVal<Env, Val>,
    V: TryFromVal<Env, Val>,
{
    env.storage().temporary().get(key)
}

/// Extends a live transient entry only when its remaining TTL is below `threshold`.
///
/// Returns `false` when `key` is absent or has already been evicted. Returns
/// `true` when the key is live; in that case Soroban performs the extension only
/// when the remaining TTL is below `threshold` and otherwise leaves the TTL
/// unchanged.
///
/// The boolean reports liveness, not whether Soroban changed the TTL. The host
/// intentionally does not expose a production API for observing an entry's TTL.
#[allow(dead_code)]
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

/// Removes a transient entry if it exists.
///
/// This operation is idempotent: removing an absent or evicted key is a no-op.
#[allow(dead_code)]
pub fn remove_transient<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    env.storage().temporary().remove(key);
}

/// Returns whether a transient key is currently live in contract storage.
///
/// Expired temporary entries are auto-evicted by Soroban and therefore return
/// `false`, just like keys that were never stored.
#[allow(dead_code)]
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
    let key = milestone_storage_key(env, contract_id);
    let milestones: Vec<Milestone> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
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
    let key = milestone_storage_key(env, contract_id);
    env.storage().persistent().set(&key, milestones);
    extend_milestone_ttl(env, contract_id);
}

pub fn milestone_storage_key(env: &Env, contract_id: u32) -> (DataKey, Symbol) {
    (
        DataKey::Contract(contract_id),
        Symbol::new(env, "milestones"),
    )
}

/// Extend TTL of the NextContractId counter.
pub fn extend_next_contract_id_ttl(env: &Env) {
    if env.storage().persistent().has(&DataKey::NextContractId) {
        env.storage().persistent().extend_ttl(
            &DataKey::NextContractId,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_TTL_LEDGERS,
        );
    }
}

/// Extend TTL of a single contract entry.
pub fn extend_contract_ttl(env: &Env, contract_id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::Contract(contract_id),
        PERSISTENT_BUMP_THRESHOLD,
        PERSISTENT_TTL_LEDGERS,
    );
}

/// Extend TTL of the milestones vector for a given contract.
pub fn extend_milestone_ttl(env: &Env, contract_id: u32) {
    env.storage().persistent().extend_ttl(
        &milestone_storage_key(env, contract_id),
        PERSISTENT_BUMP_THRESHOLD,
        PERSISTENT_TTL_LEDGERS,
    );
}

/// Extend TTL of both the contract and its milestones vector.
pub fn extend_contract_and_milestones_ttl(env: &Env, contract_id: u32) {
    extend_contract_ttl(env, contract_id);
    extend_milestone_ttl(env, contract_id);
}

/// Extend TTL for a participant contract index entry (e.g. client or freelancer id list).
pub fn extend_participant_contract_index_ttl(env: &Env, key: &crate::DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS);
}
