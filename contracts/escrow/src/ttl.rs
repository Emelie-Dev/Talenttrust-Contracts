//! Deterministic TTL / expiration policy for transient and persistent storage.

use crate::{DataKey, Error, Milestone};
use soroban_sdk::{Env, IntoVal, Symbol, TryFromVal, Val, Vec};

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
        &milestone_storage_key(env, contract_id),
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
