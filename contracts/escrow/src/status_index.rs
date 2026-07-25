//! Status index maintenance for the `list_contracts_by_status` reader.
//!
//! Each `DataKey::StatusIndex(status_code)` entry stores a `Vec<u32>` of contract
//! IDs currently carrying that lifecycle state. The vectors are updated on every
//! status transition so operators can enumerate all contracts in a given state
//! without scanning the entire ID space.
//!
//! ## Consistency guarantee
//! Every mutating entrypoint that changes `contract.status` must call
//! `update_status_index` **after** writing the new `Contract` record to storage.
//! The helper removes the contract id from the old status bucket and appends it
//! to the new one in a single storage round-trip per bucket.
//!
//! ## Storage
//! Keys: `DataKey::StatusIndex(status_code: u32)` stored in `persistent()`.
//! TTL is extended to `PERSISTENT_TTL_LEDGERS` on every write.
//!
//! ## Participant index
//! `DataKey::ParticipantContracts(address, role)` where role 0 = client,
//! role 1 = freelancer. Appended once at contract creation; never removed
//! (contract IDs remain in the participant list for their lifetime).

use crate::{ttl, ContractStatus, DataKey};
use soroban_sdk::{Env, Vec};

/// Maximum ids returned in a single paginated query.
pub const MAX_PAGE_LIMIT: u32 = 50;

/// Convert a `ContractStatus` variant to its stable u32 code.
///
/// These codes must not change; they are part of the on-chain storage schema.
pub fn status_code(status: &ContractStatus) -> u32 {
    match status {
        ContractStatus::Created => 0,
        ContractStatus::Accepted => 1,
        ContractStatus::Funded => 2,
        ContractStatus::Completed => 3,
        ContractStatus::Disputed => 4,
        ContractStatus::Cancelled => 5,
        ContractStatus::Refunded => 6,
        ContractStatus::PartiallyFunded => 7,
    }
}

/// Load the index vector for `status` (empty vec if absent).
fn load_index(env: &Env, key: &DataKey) -> Vec<u32> {
    env.storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Persist the index vector and extend its TTL.
fn save_index(env: &Env, key: &DataKey, ids: &Vec<u32>) {
    env.storage().persistent().set(key, ids);
    ttl::extend_status_index_ttl(env, key);
}

/// Remove `contract_id` from the index bucket for `old_status`.
fn remove_from_index(env: &Env, old_status: &ContractStatus, contract_id: u32) {
    let key = DataKey::StatusIndex(status_code(old_status));
    let mut ids: Vec<u32> = load_index(env, &key);
    // Find and remove the id (linear scan — index vecs are bounded by NextContractId).
    let mut found_at: Option<u32> = None;
    for i in 0..ids.len() {
        if ids.get(i).unwrap() == contract_id {
            found_at = Some(i);
            break;
        }
    }
    if let Some(pos) = found_at {
        ids.remove(pos);
        save_index(env, &key, &ids);
    }
}

/// Append `contract_id` to the index bucket for `new_status`.
fn append_to_index(env: &Env, new_status: &ContractStatus, contract_id: u32) {
    let key = DataKey::StatusIndex(status_code(new_status));
    let mut ids: Vec<u32> = load_index(env, &key);
    ids.push_back(contract_id);
    save_index(env, &key, &ids);
}

/// Transition `contract_id` from `old_status` to `new_status` in the index.
///
/// Call this **after** the `Contract` record has been written with the new status.
pub fn update_status_index(
    env: &Env,
    contract_id: u32,
    old_status: &ContractStatus,
    new_status: &ContractStatus,
) {
    if old_status == new_status {
        return;
    }
    remove_from_index(env, old_status, contract_id);
    append_to_index(env, new_status, contract_id);
}

/// Register `contract_id` in the initial status bucket (called on creation).
pub fn index_new_contract(env: &Env, contract_id: u32, initial_status: &ContractStatus) {
    append_to_index(env, initial_status, contract_id);
}

/// Register `contract_id` in the participant index for both client and freelancer.
/// Role 0 = client, role 1 = freelancer.
pub fn index_participant(env: &Env, contract_id: u32, address: &soroban_sdk::Address, role: u32) {
    let key = DataKey::ParticipantContracts(address.clone(), role);
    let mut ids: Vec<u32> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    ids.push_back(contract_id);
    env.storage().persistent().set(&key, &ids);
    ttl::extend_participant_contract_index_ttl(env, &key);
}

/// Returns a paginated slice of contract IDs for `status`.
///
/// `start` is the zero-based index into the stored list (not a contract ID).
/// `limit` is capped at [`MAX_PAGE_LIMIT`].
pub fn list_by_status(env: &Env, status: &ContractStatus, start: u32, limit: u32) -> Vec<u32> {
    let capped = limit.min(MAX_PAGE_LIMIT);
    let key = DataKey::StatusIndex(status_code(status));
    let ids: Vec<u32> = load_index(env, &key);

    // Extend TTL on read so active dashboards keep the index alive.
    if env.storage().persistent().has(&key) {
        ttl::extend_status_index_ttl(env, &key);
    }

    let total = ids.len();
    if start >= total || capped == 0 {
        return Vec::new(env);
    }

    let end = (start + capped).min(total);
    let mut page: Vec<u32> = Vec::new(env);
    for i in start..end {
        page.push_back(ids.get(i).unwrap());
    }
    page
}

/// Returns a paginated slice of contract IDs for `(participant, role)`.
///
/// Role 0 = client, role 1 = freelancer. `limit` is capped at [`MAX_PAGE_LIMIT`].
pub fn list_by_participant(
    env: &Env,
    participant: &soroban_sdk::Address,
    role: u32,
    start: u32,
    limit: u32,
) -> Vec<u32> {
    let capped = limit.min(MAX_PAGE_LIMIT);
    let key = DataKey::ParticipantContracts(participant.clone(), role);
    let ids: Vec<u32> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    if env.storage().persistent().has(&key) {
        ttl::extend_participant_contract_index_ttl(env, &key);
    }

    let total = ids.len();
    if start >= total || capped == 0 {
        return Vec::new(env);
    }

    let end = (start + capped).min(total);
    let mut page: Vec<u32> = Vec::new(env);
    for i in start..end {
        page.push_back(ids.get(i).unwrap());
    }
    page
}
