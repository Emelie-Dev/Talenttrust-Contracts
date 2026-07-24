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
}
