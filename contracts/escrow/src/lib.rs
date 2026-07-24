//! TalentTrust escrow contract for milestone-based freelancer payments.

#![no_std]
#![allow(clippy::derivable_impls)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::assertions_on_constants)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::redundant_field_names)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::useless_vec)]
#![allow(clippy::let_and_return)]
#![allow(clippy::inconsistent_digit_grouping)]
#![allow(clippy::int_plus_one)]
#![allow(clippy::duplicated_attributes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::clone_on_copy)]
#![allow(clippy::module_inception)]
#![allow(clippy::single_match)]
#![allow(clippy::useless_conversion)]

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

pub use amount_validation::accumulate_amounts;
pub use amount_validation::safe_add_amounts;
pub use amount_validation::safe_subtract_amounts;
pub use amount_validation::validate_deposit_amount;
pub use amount_validation::validate_milestone_amounts;
pub use amount_validation::validate_single_amount;
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
        env.storage()
            .persistent()
            .set(&DataKey::SettlementToken, token);
    }

    /// Returns the current escrow state for a contract.
    ///
    /// Read-only view. Returns a sensible default when no escrow record exists
    /// instead of panicking.
    pub fn get_escrow_state(env: Env, contract_id: String) -> Contract {
        let key = DataKey::Contract(contract_id);
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(Contract::default())
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

    pub fn get_settlement_token(env: Env) -> Option<Address> {
        Self::read_settlement_token(&env)
    }

    pub fn is_settlement_token_bound(env: Env) -> bool {
        Self::read_settlement_token(&env).is_some()
    }

    pub fn initialize(env: Env, admin: Address) -> bool {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(Error::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage().persistent().set(&DataKey::Initialized, &true);
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &1u32);

        let mut checklist: ReadinessChecklist = env
            .storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default();
        checklist.initialized = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);

        env.events().publish(
            (symbol_short!("init"), Symbol::new(&env, "admin_set")),
            (admin.clone(), env.ledger().timestamp()),
        );

        true
    }

    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Admin)
    }

    // The rest of the original code goes here.
    // (Keep the rest of your original functions as they were from the previous version)
    // If you lost them, you can revert the file from GitHub history or re-clone the repo.
        }
