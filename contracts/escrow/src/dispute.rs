//! Dispute payout arithmetic, final-status helpers, and dispute-record storage.
//!
//! This module is the single owner of dispute-related helpers:
//!
//! - [`resolution_payouts`] computes how the available escrow balance should be
//!   split for a [`DisputeResolution`].
//! - [`final_status_after_resolution`] decides whether dispute settlement leaves
//!   the contract as [`ContractStatus::Completed`] or [`ContractStatus::Refunded`].
//! - [`write_dispute_record`], [`update_dispute_record`], [`read_dispute_record`],
//!   and [`has_dispute_record`] own persistence of the
//!   [`DisputeRecord`](crate::DisputeRecord) under
//!   [`DataKey::Dispute`](crate::DataKey::Dispute).
//!
//! The root `raise_dispute` / `resolve_dispute` entrypoints live in
//! `contracts/escrow/src/lib.rs` and delegate record persistence to this
//! module so the read view ([`crate::Escrow::get_dispute_record`]) can read
//! stored values without recomputation.

use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::{
    safe_add_amounts, Contract, ContractStatus, DataKey, DisputeRecord, DisputeResolution,
    DisputeSplit, Error, Escrow, EscrowArgs, EscrowClient,
};

// ---------------------------------------------------------------------------
// resolution_payouts: pure arithmetic for dispute payout calculations
// ---------------------------------------------------------------------------

/// Compute the payout split for a dispute resolution.
///
/// Returns `(client_payout, freelancer_payout)` where both values are non-negative
/// and sum to the available balance. The available balance is computed as:
/// `available = funded_amount - released_amount - refunded_amount`.
///
/// # Errors
/// - `AccountingInvariantViolated` if available would be negative (corrupted state)
/// - `PotentialOverflow` if intermediate calculations overflow
/// - `InvalidDisputeSplit` for Split variant with negative legs or non-conserving sum
pub fn resolution_payouts(
    contract: &Contract,
    resolution: &DisputeResolution,
) -> Result<(i128, i128), Error> {
    let available = contract
        .funded_amount
        .checked_sub(contract.released_amount)
        .and_then(|value| value.checked_sub(contract.refunded_amount))
        .ok_or(Error::AccountingInvariantViolated)?;
    if available < 0 {
        return Err(Error::AccountingInvariantViolated);
    }

    match resolution {
        DisputeResolution::FullRefund => Ok((available, 0)),
        DisputeResolution::PartialRefund => {
            // freelancer gets floor(available * 30 / 100), client gets remainder
            let freelancer_payout = available
                .checked_mul(30)
                .and_then(|value| value.checked_div(100))
                .ok_or(Error::PotentialOverflow)?;
            Ok((available - freelancer_payout, freelancer_payout))
        }
        DisputeResolution::FullPayout => Ok((0, available)),
        DisputeResolution::Split(split) => {
            if split.client_amount < 0 || split.freelancer_amount < 0 {
                return Err(Error::InvalidDisputeSplit);
            }
            // Issue #572: Reject split resolution whose components are individually within but jointly exceed balance
            if split.client_amount > available || split.freelancer_amount > available {
                return Err(Error::InvalidDisputeSplit);
            }
            let total = safe_add_amounts(split.client_amount, split.freelancer_amount)
                .ok_or(Error::PotentialOverflow)?;
            if total > available || total != available {
                return Err(Error::InvalidDisputeSplit);
            }
            Ok((split.client_amount, split.freelancer_amount))
        }
    }
}

/// Determine the final contract status after dispute resolution.
///
/// Returns `Refunded` only when the full deposit has been refunded.
/// Otherwise returns `Completed`.
pub fn final_status_after_resolution(contract: &Contract) -> ContractStatus {
    if contract.refunded_amount == contract.funded_amount {
        ContractStatus::Refunded
    } else {
        ContractStatus::Completed
    }
}

// ---------------------------------------------------------------------------
// DisputeRecord storage helpers (issue #795: read-only view of dispute state)
// ---------------------------------------------------------------------------

/// Storage key for the persistent [`DisputeRecord`] of `contract_id`.
pub(crate) fn dispute_record_key(contract_id: u32) -> DataKey {
    DataKey::Dispute(contract_id)
}

/// Persist a fresh [`DisputeRecord`] for `contract_id` capturing the raiser
/// side of the lifecycle. The resolution-side fields are left as `None` so a
/// single struct shape covers open and resolved states.
///
/// Called from `Escrow::raise_dispute` after the contract has been validated,
/// transitioned to [`ContractStatus::Disputed`], and persisted. The
/// companion contract record is bumped first by the caller so the dispute
/// record inherits the same eviction horizon through
/// [`extend_dispute_record_ttl`].
pub fn write_dispute_record(env: &Env, contract_id: u32, raiser: Address) {
    let record = DisputeRecord {
        raiser,
        raised_at_ledger: env.ledger().sequence(),
        raised_at_timestamp: env.ledger().timestamp(),
        resolver: None,
        resolution: None,
        resolved_at_ledger: None,
        resolved_at_timestamp: None,
    };
    env.storage()
        .persistent()
        .set(&dispute_record_key(contract_id), &record);
    extend_dispute_record_ttl(env, contract_id);
}

/// Update an existing [`DisputeRecord`] with resolver-side fields after a
/// successful `resolve_dispute`. Builds a fresh record from
/// `env.ledger().sequence()` and `env.ledger().timestamp()` to keep the
/// resolved-side timestamps consistent with the existing raise-side entries.
///
/// # Errors
/// - `Error::InvalidState` if no live dispute record exists. `raise_dispute`
///   must run successfully before `resolve_dispute` is ever called, so this
///   path is unreachable in normal operation but is fail-closed defensively.
pub fn update_dispute_record(
    env: &Env,
    contract_id: u32,
    resolver: Address,
    resolution: DisputeResolution,
) {
    let key = dispute_record_key(contract_id);
    let mut record: DisputeRecord = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| env.panic_with_error(Error::InvalidState));
    record.resolver = Some(resolver);
    record.resolution = Some(resolution);
    record.resolved_at_ledger = Some(env.ledger().sequence());
    record.resolved_at_timestamp = Some(env.ledger().timestamp());
    env.storage().persistent().set(&key, &record);
    extend_dispute_record_ttl(env, contract_id);
}

/// Cheap, non-mutating existence probe — returns `true` if a dispute record
/// for `contract_id` is currently live in persistent storage.
///
/// Identical semantics to [`crate::Escrow::has_dispute`]: storage presence is
/// the single source of truth, so an unknown contract id and a contract that
/// was never disputed both return `false`.
pub fn has_dispute_record(env: &Env, contract_id: u32) -> bool {
    env.storage()
        .persistent()
        .has(&dispute_record_key(contract_id))
}

/// Read the [`DisputeRecord`] for `contract_id`, extending TTL on a successful
/// read. Returns `None` for unknown contracts and contracts that have never
/// been disputed — both cases are intentionally indistinguishable because
/// off-chain callers should not need to disambiguate "contract does not
/// exist" from "contract has no dispute" for the read view.
///
/// Reading the record bumps its own TTL (under
/// [`DataKey::Dispute`](crate::DataKey::Dispute)) so off-chain indexers that
/// poll dispute state keep the dispute record alive without re-raising it.
/// The companion contract-record TTL is bumped via
/// [`crate::ttl::extend_contract_ttl`] because the dispute record's liveness
/// is bounded by the surrounding contract metadata lifecycle.
pub fn read_dispute_record(env: &Env, contract_id: u32) -> Option<DisputeRecord> {
    let key = dispute_record_key(contract_id);
    let record: Option<DisputeRecord> = env.storage().persistent().get(&key);
    if record.is_some() {
        extend_dispute_record_ttl(env, contract_id);
        // Also keep the contract record alive — the dispute record is most
        // useful alongside the surrounding contract metadata, so co-bumping
        // the two keys keeps the O(1) view and the broader contract state
        // on the same lifecycle.
        crate::ttl::extend_contract_ttl(env, contract_id);
    }
    record
}

/// Extend the persistent TTL of the dispute record under
/// [`DataKey::Dispute`](crate::DataKey::Dispute). Uses the standard
/// persistent-storage policy (bump below `PERSISTENT_BUMP_THRESHOLD`, extend
/// to `PERSISTENT_TTL_LEDGERS`) so dispute records share the same eviction
/// horizon as the rest of the contract metadata.
pub(crate) fn extend_dispute_record_ttl(env: &Env, contract_id: u32) {
    env.storage().persistent().extend_ttl(
        &dispute_record_key(contract_id),
        crate::ttl::PERSISTENT_BUMP_THRESHOLD,
        crate::ttl::PERSISTENT_TTL_LEDGERS,
    );
}

// ---------------------------------------------------------------------------
// raise_dispute / resolve_dispute entrypoints
// ---------------------------------------------------------------------------

// Dispute entrypoints are implemented in `contracts/escrow/src/lib.rs`.
// This module retains dispute-related helpers only.
