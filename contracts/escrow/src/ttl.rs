//! Deterministic TTL / expiration policy for transient and persistent storage.
//!
//! All TTL values are denominated in ledgers (Soroban-native, ~5s per ledger
//! on Stellar mainnet). Pending approvals and pending migrations are stored
//! in `env.storage().temporary()`; Soroban auto-evicts entries whose TTL has
//! elapsed, so `read_if_live` returns `None` for both "never set" and
//! "expired".

use crate::{DataKey, Error, Milestone};
use soroban_sdk::{Env, IntoVal, Symbol, TryFromVal, Val, Vec};

pub const LEDGERS_PER_DAY: u32 = 17_280;

pub const PENDING_APPROVAL_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 7;
pub const PENDING_APPROVAL_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY;

/// Minimum ledgers that must elapse between proposing and finalising a
/// treasury / admin rotation.  At ~5 s per ledger this is roughly 2 days,
/// giving stakeholders time to react to an unexpected proposal.
pub const ADMIN_ROTATION_MIN_DELAY_LEDGERS: u32 = LEDGERS_PER_DAY * 2;

#[allow(dead_code)]
pub const PENDING_MIGRATION_TTL_LEDGERS: u32 = LEDGERS_PER_DAY * 21;
#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn remove_transient<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    env.storage().temporary().remove(key);
}

#[allow(dead_code)]
pub fn has_transient<K>(env: &Env, key: &K) -> bool
where
    K: IntoVal<Env, Val>,
{
    env.storage().temporary().has(key)
}

/// Load the milestone vector for a contract, extending its TTL.
/// Panics with `Error::ContractNotFound` if absent.
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

/// Store the milestone vector for a contract, extending its TTL.
pub fn store_milestones(env: &Env, contract_id: u32, milestones: &Vec<Milestone>) {
    let key = milestone_storage_key(env, contract_id);
    env.storage().persistent().set(&key, milestones);
    extend_milestone_ttl(env, contract_id);
}

pub(crate) fn milestone_storage_key(env: &Env, contract_id: u32) -> (DataKey, Symbol) {
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
///
/// This is called on index writes to avoid index entries expiring during normal usage.
pub fn extend_participant_contract_index_ttl(env: &Env, key: &crate::DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS);
}
