//! TalentTrust escrow contract for milestone-based freelancer payments.

#![no_std]
#![allow(clippy::derivable_impls)]
#![allow(clippy::manual_range_contains)]
// ... (keep all your #![allow] lines)

mod amount_validation;
mod approvals;
mod deposit;
mod finalize;
mod migration;
mod status_index;
mod ttl;
mod types;
mod utils;

use crate::utils::now_seconds;
use soroban_sdk::{
    contract, contracterror, contractimpl, log, symbol_short, token, Address, Env, String, Symbol,
    Vec,
};

pub use amount_validation::*;
pub use dispute::final_status_after_resolution;
pub use dispute::resolution_payouts;
pub use migration::PendingClientMigration;
pub use ttl::{ADMIN_ROTATION_MIN_DELAY_LEDGERS, PENDING_MIGRATION_TTL_LEDGERS};
pub use types::{
    Contract, ContractBounds, ContractStatus, ContractSummary, DataKey, DepositMode,
    DisputeRecord, DisputeResolution, DisputeSplit, Error, GovernedParameters, Milestone,
    MilestoneApprovals, MilestoneSummary, PendingAdminProposal, ReadinessChecklist,
    DisputeResolution, DisputeSplit, Error, GovernedParameters, Milestone, MilestoneApprovals,
    MilestoneSummary, PendingAdminProposal, ReadinessChecklist, ReleaseAuthorization, Reputation,
    SplitAmounts, CONTRACT_SUMMARY_SCHEMA_VERSION,
};

pub const MAX_MILESTONES: u32 = 10;
pub const MAX_SINGLE_AMOUNT_STROOPS: i128 = crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS;
pub const MAX_TOTAL_ESCROW_STROOPS: i128 = MAX_SINGLE_AMOUNT_STROOPS;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationState {
    pub admin: Option<Address>,
    pub initialized: bool,
    pub paused: bool,
    pub emergency_active: bool,
}

#[contract]
pub struct Escrow;

mod create_contract;
mod dispute;
mod governance;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EscrowError {
    // your existing errors...
}

impl Escrow {
    // your existing helpers...
}

#[contractimpl]
impl Escrow {
    // ALL your existing functions stay here...

    /// Returns the current authorization state for the contract.
    ///
    /// Read-only view. Does not mutate storage.
    /// Returns a sensible default when authorization is unset.
    pub fn get_authorization_state(env: Env) -> AuthorizationState {
        let admin = Self::get_admin(env.clone());
        let initialized = env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false);

        AuthorizationState {
            admin,
            initialized,
            paused: Self::is_paused(&env),
            emergency_active: Self::is_emergency_active(&env),
        }
    }

    /// Returns a paginated list of contract IDs currently in `status`.
    ///
    /// Backed by a maintained `DataKey::StatusIndex(status_code)` persistent vector
    /// that is updated on every status transition. Operators can cheaply answer
    /// "which escrows are currently Disputed or Funded" without scanning the full
    /// ID space.
    ///
    /// # Arguments
    /// * `env`    — The Soroban environment
    /// * `status` — The lifecycle state to enumerate
    /// * `start`  — Zero-based offset into the index (not a contract ID)
    /// * `limit`  — Maximum IDs to return; capped at `MAX_PAGE_LIMIT` (50)
    ///
    /// # Returns
    /// A `Vec<u32>` of contract IDs in insertion order. Empty when `start` is
    /// out of range or the index has no entries for `status`.
    ///
    /// # Auth
    /// Auth-free and read-only; extends the index TTL on read.
    pub fn list_contracts_by_status(
        env: Env,
        status: ContractStatus,
        start: u32,
        limit: u32,
    ) -> Vec<u32> {
        status_index::list_by_status(&env, &status, start, limit)
    }

    /// Returns a paginated list of contract IDs for a participant.
    ///
    /// Backed by `DataKey::ParticipantContracts(address, role)` written once
    /// at contract creation.
    ///
    /// * `role` — `0` for client, `1` for freelancer
    /// * `start` — Zero-based offset into the list
    /// * `limit` — Maximum IDs to return; capped at `MAX_PAGE_LIMIT` (50)
    ///
    /// # Auth
    /// Auth-free and read-only; extends the index TTL on read.
    pub fn list_contracts_by_participant(
        env: Env,
        participant: Address,
        role: u32,
        start: u32,
        limit: u32,
    ) -> Vec<u32> {
        status_index::list_by_participant(&env, &participant, role, start, limit)
    }
}
