//! TalentTrust escrow contract for milestone-based freelancer payments.
//!
//! The crate root exposes the Soroban contract and still owns several public
//! entrypoints directly: initialization, settlement-token binding, deposits,
//! milestone release/refund/cancel flows, reputation, work evidence, protocol
//! fee withdrawal, and dispute entrypoints. Supporting modules keep reusable
//! validation, storage, governance, and lifecycle helpers close to the paths
//! that use them.
//!
//! ## Escrow source tree map
//!
//! (your original documentation table unchanged)

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
    contract, contracterror, contractimpl, contracttype, log, symbol_short, token, Address, Env, String, Symbol,
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

// ── AuthorizationState for read-only view (#820) ─────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationState {
    pub release_authorization: ReleaseAuthorization,
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
    InvalidParticipant = 1,
    EmptyMilestones = 2,
    InvalidMilestoneAmount = 3,
    InvalidDepositAmount = 4,
    InvalidMilestone = 5,
    ContractNotFound = 6,
    EmptyRefundRequest = 7,
    DuplicateMilestoneInRefund = 8,
    AlreadyReleased = 9,
    AlreadyRefunded = 10,
    InsufficientFunds = 11,
    AlreadyInitialized = 12,
    InsufficientAccumulatedFees = 13,
    NotInitialized = 14,
    UnauthorizedRole = 15,
    ContractPaused = 16,
    EmergencyActive = 17,
    InvalidState = 18,
    InvalidRating = 19,
    SelfRating = 20,
    ReputationAlreadyIssued = 21,
    NotCompleted = 22,
    FreelancerMismatch = 23,
    InvalidStatusTransition = 24,
    ArbiterRequired = 25,
    InvalidDisputeSplit = 26,
    AccountingInvariantViolated = 27,
    PotentialOverflow = 28,
    AlreadyFinalized = 29,
    AmountMustBePositive = 30,
    SettlementTokenNotConfigured = 31,
    SettlementTokenAlreadyBound = 32,
    TotalCapExceeded = 33,
    TooManyMilestones = 34,
    MissingArbiter = 35,
    InvalidArbiter = 36,
    ContractCancelled = 37,
    ContractRefunded = 38,
    InvalidSettlementToken = 39,
    SettlementTokenIsSelf = 40,
    SettlementTokenIsAdmin = 41,
    EmptyComment = 42,
    CommentTooLong = 43,
}

impl Escrow {
    pub(crate) fn read_settlement_token(env: &Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::SettlementToken)
    }

    pub(crate) fn write_settlement_token(env: &Env, token: &Address) {
        env.storage().persistent().set(&DataKey::SettlementToken, token);
    }

    pub(crate) fn require_initialized(env: &Env) {
        if !env.storage().persistent().get(&DataKey::Initialized).unwrap_or(false) {
            env.panic_with_error(EscrowError::NotInitialized);
        }
    }
}

#[contractimpl]
impl Escrow {
    pub fn bind_settlement_token(env: Env, admin: Address, token: Address) -> bool {
        Self::require_initialized(&env);
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));

        if admin != stored_admin {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }
        admin.require_auth();

        if Self::read_settlement_token(&env).is_some() {
            env.panic_with_error(EscrowError::SettlementTokenAlreadyBound);
        }

        if token == env.current_contract_address() {
            env.panic_with_error(EscrowError::SettlementTokenIsSelf);
        }

        if token == stored_admin {
            env.panic_with_error(EscrowError::SettlementTokenIsAdmin);
        }

        let token_client = token::Client::new(&env, &token);
        let _probe: i128 = token_client.balance(&env.current_contract_address());

        Self::write_settlement_token(&env, &token);

        env.events().publish(
            (Symbol::new(&env, "settlement_token_bound"),),
            (admin, token, env.ledger().timestamp()),
        );
        true
    }

    #[deprecated(note = "Use bind_settlement_token instead.")]
    pub fn set_settlement_token(env: Env, admin: Address, token: Address) -> bool {
        Self::bind_settlement_token(env, admin, token)
    }

    pub fn get_settlement_token(env: Env) -> Option<Address> {
        Self::read_settlement_token(&env)
    }

    pub fn is_settlement_token_bound(env: Env) -> bool {
        Self::read_settlement_token(&env).is_some()
    }

    pub fn initialize(env: Env, admin: Address) -> bool {
        if env.storage().persistent().get::<_, bool>(&DataKey::Initialized).unwrap_or(false) {
            env.panic_with_error(Error::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage().persistent().set(&DataKey::Initialized, &true);
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::NextContractId, &1u32);

        let mut checklist: ReadinessChecklist = env.storage().persistent().get(&DataKey::ReadinessChecklist).unwrap_or_default();
        checklist.initialized = true;
        env.storage().persistent().set(&DataKey::ReadinessChecklist, &checklist);

        env.events().publish(
            (symbol_short!("init"), Symbol::new(&env, "admin_set")),
            (admin.clone(), env.ledger().timestamp()),
        );

        true
    }

    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Admin)
    }

    /// Read-only view exposing the current authorization state for a contract.
    /// Added for issue #820.
    pub fn get_authorization_state(env: Env, contract_id: u32) -> AuthorizationState {
        let default = AuthorizationState {
            release_authorization: ReleaseAuthorization::ClientOnly,
        };

        env.storage()
            .persistent()
            .get::<_, Contract>(&DataKey::Contract(contract_id))
            .map(|contract| AuthorizationState {
                release_authorization: contract.release_authorization,
            })
            .unwrap_or(default)
    }
}

/// Test fixtures and suites are compiled only for native test builds, never wasm.
#[cfg(test)]
mod test;
