use crate::types::DataKey;
use soroban_sdk::Env;

/// Current on-chain storage layout version for escrow state.
///
/// Version `0` represents the legacy layout that predates the storage marker.
/// The migration routine upgrades that layout in place by stamping the marker
/// while preserving the existing persisted data under the current keys.
pub const ESCROW_STORAGE_VERSION: u32 = 1;

pub(crate) fn ensure_storage_version(env: &Env) {
    let stored_version = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKey::StorageVersion)
        .unwrap_or(0);

    if stored_version == ESCROW_STORAGE_VERSION {
        return;
    }

    migrate_storage_to_current(env, stored_version);
}

pub(crate) fn initialize_storage_version(env: &Env) {
    let stored_version = env
        .storage()
        .persistent()
        .get::<_, u32>(&DataKey::StorageVersion)
        .unwrap_or(0);

    if stored_version < ESCROW_STORAGE_VERSION {
        migrate_storage_to_current(env, stored_version);
    }
}

fn migrate_storage_to_current(env: &Env, from_version: u32) {
    match from_version {
        0 => migrate_v0_to_v1(env),
        _ => env
            .storage()
            .persistent()
            .set(&DataKey::StorageVersion, &ESCROW_STORAGE_VERSION),
    }
}

fn migrate_v0_to_v1(env: &Env) {
    // Legacy layouts already persisted the escrow state under the current keys.
    // This migration is therefore a no-op on the data itself: it only stamps
    // the storage version so future reads can follow the versioned path.
    env.storage()
        .persistent()
        .set(&DataKey::StorageVersion, &ESCROW_STORAGE_VERSION);
}
