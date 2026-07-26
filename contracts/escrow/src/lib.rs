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
//! | Source | Responsibility | Storage keys owned or touched |
//! | --- | --- | --- |
//! | `lib.rs` | Contract wrapper plus root entrypoints for setup, custody, money movement, reads, reputation, work evidence, pause/emergency, fee withdrawal, and dispute orchestration. | `DataKey::Initialized`, `Admin`, `Paused`, `Emergency`, `ReadinessChecklist`, `Contract(id)`, `(Contract(id), "milestones")`, `MilestoneApprovals`, `AccumulatedProtocolFees`, `ReputationIssued`, `PendingReputationCredits`, `Reputation`, `ReputationComment` |
//! | `settlement` | Typed storage keys and read/write helpers for settlement entries (token binding, finalization records). | `DataKey::SettlementToken`, `DataKey::Finalization(contract_id)`; delegates to `settlement` helpers. |
//! | `amount_validation` | Stateless validation and checked arithmetic for stroop amounts and milestone totals. | None directly; callers write validated amounts to `Contract(id)` and milestone vectors. |
//! | `approvals` | Temporary milestone release approvals and release-authorization checks. | Temporary `DataKey::MilestoneApprovals(contract_id, milestone_index)`; reads `Contract(id)` and `(Contract(id), "milestones")`. |
//! | `deposit` | Deposit preflight and post-transfer accounting used by `deposit_funds`. | `DataKey::Contract(contract_id)` and `(DataKey::Contract(contract_id), "milestones")`. |
//! | `finalize` | Immutable finalization records, finalization guards, and final contract summaries. | `DataKey::Finalization(contract_id)`; reads `Contract(id)`, `(Contract(id), "milestones")`, `Paused`, and `Emergency`. |
//! | `migration` | Client migration proposals, acceptance checks, cancellation, and pending-migration reads. | Temporary `DataKey::PendingClientMigration(contract_id)`; reads and updates `DataKey::Contract(contract_id)`. |
//! | `ttl` | TTL constants plus helpers for temporary and persistent storage renewal. | Extends caller-provided keys, especially `Contract(id)`, `(Contract(id), "milestones")`, `NextContractId`, participant indexes, approvals, and migrations. |
//! | `types` | Shared Soroban types, error enums, summaries, governance records, dispute records, and the canonical `DataKey` enum. | Declares storage key schema only; does not access storage itself. |
//! | `utils` | Small deterministic helpers shared by entrypoints, currently ledger timestamp access. | None. |
//! | `create_contract` | Contract creation, participant/milestone validation, ID allocation, and creation events. | `DataKey::Contract(id)`, `(DataKey::Contract(id), "milestones")`, `NextContractId`, and `GovernedParameters`. |
//! | `dispute` | Pure dispute payout arithmetic and final-status selection for dispute resolution. | None directly; root dispute entrypoints update `DataKey::Contract(contract_id)`. |
//! | `governance` | Admin-controlled protocol fee, governed parameter, readiness, and admin-rotation entrypoints. | `DataKey::Admin`, `ProtocolFeeBps`, `PendingAdmin`, `GovernedParameters`, and `ReadinessChecklist`. |
//!
//! Generate this map with `cargo doc -p escrow --no-deps` and open
//! `target/doc/escrow/index.html`.
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
mod settlement;
mod ttl;
mod types;
mod utils;

use crate::utils::now_seconds;
use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, log, symbol_short, token,
    Address, Env, String, Symbol, Vec,
};

pub use amount_validation::accumulate_amounts;
pub use amount_validation::checked_available_balance;
pub use amount_validation::safe_add_amounts;
pub use amount_validation::safe_subtract_amounts;
pub use amount_validation::validate_deposit_amount;
pub use amount_validation::validate_milestone_amounts;
pub use amount_validation::validate_single_amount;
pub use amount_validation::MAX_SINGLE_AMOUNT_STROOPS;
pub use dispute::final_status_after_resolution;
pub use dispute::resolution_payouts;
pub use migration::PendingClientMigration;
pub use ttl::{ADMIN_ROTATION_MIN_DELAY_LEDGERS, PENDING_MIGRATION_TTL_LEDGERS};

pub use types::{
    Contract, ContractBounds, ContractStatus, ContractSummary, DataKey, DepositMode,
    DisputeResolution, DisputeSplit, Error, GovernedParameters, Milestone,
    MilestoneEntry, MilestoneSummary, PendingAdminProposal, ReadinessChecklist,
    ReleaseAuthorization, Reputation, SplitAmounts, StorageKey,
    CONTRACT_SUMMARY_SCHEMA_VERSION,
};

/// Default maximum number of milestones allowed per contract.
pub const DEFAULT_MAX_MILESTONES: u32 = 10;

/// Default hard cap on the total escrow value per contract, in stroops.
pub const DEFAULT_MAX_TOTAL_ESCROW_STROOPS: i128 = 10_000_000_000_000;

/// Backward-compatible alias for the default max milestones.
pub const MAX_MILESTONES: u32 = DEFAULT_MAX_MILESTONES;

/// Backward-compatible alias for the default max escrow stroops.
pub const MAX_TOTAL_ESCROW_STROOPS: i128 = DEFAULT_MAX_TOTAL_ESCROW_STROOPS;

/// Maximum amount allowed for a single milestone, in stroops. Brought into
/// crate-root scope from `amount_validation` because `get_bounds` reads it
/// directly; without this re-export the crate does not compile.
pub const MAX_SINGLE_AMOUNT_STROOPS: i128 = crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS;

/// Absolute minimum for the max milestones setting.
pub const MIN_MAX_MILESTONES: u32 = 1;

/// Absolute maximum for the max milestones setting.
pub const MAX_MAX_MILESTONES: u32 = 100;

/// Maximum entries per page returned by paginated views.
pub const PAGE_CEILING: u32 = 100;

/// Absolute minimum for the max escrow stroops setting (0.01 XLM).
pub const MIN_MAX_ESCROW_STROOPS: i128 = 1_000_000;

/// Shared upper bound on the number of entries any paginated read view
/// (e.g. [`Escrow::get_milestones_page`], [`Escrow::get_contracts_page`])
/// returns in a single call, regardless of the caller-supplied `limit`.
///
/// This keeps per-call host resource usage predictable for indexers and
/// UIs, independent of how large the underlying collection grows.
pub const PAGE_CEILING: u32 = 50;

pub const MAINNET_PROTOCOL_VERSION: u32 = 1u32;
pub const MAINNET_MAX_TOTAL_ESCROW_PER_CONTRACT_STROOPS: i128 = 1_000_000_000_000_000i128;

// ─── Contract data ────────────────────────────────────────────────────────────

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowContractData {
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Option<Address>,
    pub milestones: Vec<i128>,
    pub status: ContractStatus,
    pub total_deposited: i128,
    pub released_amount: i128,
    pub refunded_amount: i128,
    pub reputation_issued: bool,
}

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationRecord {
    pub completed_contracts: u32,
    pub total_rating: i128,
    pub last_rating: i128,
}

impl Default for ReputationRecord {
    fn default() -> Self {
        ReputationRecord {
            completed_contracts: 0,
            total_rating: 0,
            last_rating: 0,
        }
    }
}

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MainnetReadinessInfo {
    pub initialized: bool,
    pub governed_params_set: bool,
    pub emergency_controls_enabled: bool,
    pub caps_set: bool,
    pub protocol_version: u32,
    pub max_escrow_total_stroops: i128,
}

#[contract]
pub struct Escrow;

mod create_contract;
mod dispute;
mod governance;

/// Governance-level errors for admin-gated operations.
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
    /// Returned by lifecycle entrypoints when `initialize` has not been called.
    ///
    /// All money-flow operations require initialization so the admin-controlled
    /// safety rails (pause, emergency controls, protocol fees) are always in
    /// scope before any funds can move.
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
    /// No settlement token has been bound for custody transfers.
    SettlementTokenNotConfigured = 31,
    /// A settlement token has already been bound.
    SettlementTokenAlreadyBound = 32,
    /// The sum of milestone amounts exceeded the configured maximum or overflowed.
    TotalCapExceeded = 33,
    /// Too many milestones were provided.
    TooManyMilestones = 34,
    /// An arbiter was required by the release authorization mode but not provided.
    MissingArbiter = 35,
    /// The provided arbiter is invalid (same as client or freelancer).
    InvalidArbiter = 36,
    /// Contract is cancelled and must not accept further value-moving operations.
    ContractCancelled = 37,
    /// Contract has been refunded and is terminal for value-moving operations.
    ContractRefunded = 38,
    /// The address supplied as settlement token is not a valid token contract.
    /// The pre-bind probe called `token::Client::balance` against the escrow
    /// contract address and the call panicked — the address does not implement
    /// the SAC token interface.
    InvalidSettlementToken = 39,
    /// The address supplied as settlement token is the escrow contract itself.
    /// Binding self would create a circular custody reference and brick all
    /// transfer paths.
    SettlementTokenIsSelf = 40,
    /// The address supplied as settlement token is the escrow admin.
    /// Binding the admin as the custody asset conflates governance authority
    /// with the settlement token role.
    SettlementTokenIsAdmin = 41,
    /// Reputation feedback comment was empty.
    EmptyComment = 42,
    /// Reputation feedback comment exceeded the 200-character maximum.
    CommentTooLong = 43,
    /// Configured limit is out of allowed range.
    LimitOutOfRange = 44,
}

impl Escrow {
    pub(crate) fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
        let next_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1);
        if contract_id == 0 || contract_id >= next_id {
            env.panic_with_error(EscrowError::InvalidContractId);
        }
    }

    /// Get the settlement token address from the canonical `DataKey` binding.
    pub(crate) fn read_settlement_token(env: &Env) -> Option<Address> {
        settlement::read_settlement_token(env)
    }

    pub(crate) fn write_settlement_token(env: &Env, token: &Address) {
        settlement::write_settlement_token(env, token);
    }

    pub(crate) fn require_initialized(env: &Env) {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(Error::NotInitialized);
        }
    }

    pub(crate) fn is_initialized(env: &Env) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
    }

    pub(crate) fn require_not_paused(env: &Env) {
        if env.storage().persistent().get::<_, bool>(&DataKey::Paused).unwrap_or(false) {
            env.panic_with_error(EscrowError::ContractPaused);
        }
        if env.storage().persistent().get::<_, bool>(&DataKey::Emergency).unwrap_or(false) {
            env.panic_with_error(EscrowError::EmergencyActive);
        }
    }

    pub(crate) fn require_not_finalized(env: &Env, contract_id: u32) {
        if env.storage().persistent().has(&DataKey::Finalization(contract_id)) {
            env.panic_with_error(EscrowError::AlreadyFinalized);
        }
    }

    pub(crate) fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
        if contract_id == 0 {
            env.panic_with_error(EscrowError::InvalidContractId);
        }
    }

    /// Validate that a contract ID is within acceptable bounds.
    pub(crate) fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
        if contract_id == 0 {
            env.panic_with_error(Error::InvalidContractId);
        }
    }
}

#[contractimpl]
impl Escrow {
    pub fn bind_settlement_token(env: Env, admin: Address, token: Address) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
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

    pub fn get_bounds(_env: Env) -> ContractBounds {
        ContractBounds {
            max_milestones: MAX_MILESTONES,
            max_single_milestone_stroops: crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS,
            max_total_escrow_stroops: MAX_TOTAL_ESCROW_STROOPS,
            max_fee_bps: MAX_FEE_BPS,
        }
    }

    /// Returns the current mainnet readiness checklist.
    ///
    /// The checklist tracks critical configuration steps that must be completed
    /// before the escrow contract is considered ready for mainnet production:
    ///
    /// - **`initialized`**: Flipped to `true` when `initialize` completes successfully.
    ///   Ensures that an admin has been bound to the contract.
    /// - **`governed_params_set`**: Flipped to `true` when governance/protocol parameters
    ///   (such as fees and maximum caps) are configured. Flipped during `initialize_protocol_governance`
    ///   or parameter updates.
    /// - **`emergency_controls_enabled`**: Flipped to `true` when emergency pause controls are exercised
    ///   for the first time (via `activate_emergency_pause`). This verifies the operator has functioning
    ///   emergency access.
    ///
    /// # Implications for a Clean Deploy
    /// Activating the emergency pause to flip the `emergency_controls_enabled` flag leaves the contract
    /// in a paused state. To complete a clean deploy and allow normal operations, the operator must
    /// subsequently call `resolve_emergency` to unpause the contract.
    pub fn get_readiness_checklist(env: Env) -> ReadinessChecklist {
        env.storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default()
    }

    /// Creates a new escrow contract with the specified client, freelancer, and milestone amounts.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `client` - The address of the client funding the contract
    /// * `freelancer` - The address of the freelancer performing the work
    /// * `arbiter` - Optional arbiter address for dispute resolution
    /// * `milestones` - Vector of milestone amounts (in stroops)
    /// * `release_authorization` - Authorization mode for milestone releases
    ///
    /// # Returns
    /// The unique contract ID
    ///
    /// # Errors
    /// * `InvalidParticipants` - If client and freelancer are the same address
    /// * `EmptyMilestones` - If no milestones are provided
    /// * `InvalidMilestoneAmount` - If any milestone amount is <= 0
    pub fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
        if contract_id == 0 {
            env.panic_with_error(EscrowError::ContractNotFound);
        }
        let next_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1);
        if contract_id >= next_id {
            env.panic_with_error(EscrowError::ContractNotFound);
        }
    }

    pub fn create_contract(
        env: Env,
        client: Address,
        freelancer: Address,
        arbiter: Option<Address>,
        milestones: Vec<i128>,
        release_authorization: ReleaseAuthorization,
    ) -> u32 {
        create_contract::execute_create_contract(
            env,
            client,
            freelancer,
            arbiter,
            milestones,
            release_authorization,
        )
    }
    /// Pull the settlement-token deposit from the client into the escrow contract address.
    ///
    /// Executes `SAC::transfer(from: client, to: escrow_address, amount)` and advances
    /// status from `Created` to `Funded` once the full milestone sum has been deposited.
    /// Requires `bind_settlement_token` to have been called first; panics with
    /// `SettlementTokenNotConfigured` otherwise.
    ///
    /// See [`docs/escrow/sac-custody.md`](../../../docs/escrow/sac-custody.md) for the
    /// full custody model and accounting invariant.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `caller` - The address of the caller (must be the client)
    /// * `amount` - The amount to deposit (in stroops)
    ///
    /// # Returns
    /// `true` if deposit was successful
    ///
    /// # Errors
    /// * `SettlementTokenNotConfigured` - If `bind_settlement_token` has not been called
    /// * `AmountMustBePositive` - If amount is <= 0
    /// * `ContractNotFound` - If contract doesn't exist
    /// * `InvalidState` - If contract is not in Created state
    /// * `UnauthorizedRole` - If caller is not the client
    pub fn deposit_funds(env: Env, contract_id: u32, caller: Address, amount: i128) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);

        let validated = deposit::validate_deposit(&env, contract_id, &caller, amount);

        let token_client = Self::require_settlement_token(&env);
        token_client.transfer(&caller, &env.current_contract_address(), &amount);

        deposit::apply_validated_deposit(&env, contract_id, caller, validated)
    }

    pub fn finalize_contract(env: Env, contract_id: u32, finalizer: Address) -> bool {
        finalize::finalize_contract_impl(&env, contract_id, finalizer)
    }

    pub fn get_finalization_record(
        env: Env,
        contract_id: u32,
    ) -> Option<finalize::FinalizationRecord> {
        finalize::get_finalization_record_impl(&env, contract_id)
    }

    pub fn propose_client_migration(
        env: Env,
        contract_id: u32,
        current_client: Address,
        new_client: Address,
    ) -> bool {
        Self::require_not_paused(&env);
        migration::propose_client_migration_impl(&env, contract_id, current_client, new_client)
    }

    pub fn accept_client_migration(env: Env, contract_id: u32, new_client: Address) -> bool {
        Self::require_not_paused(&env);
        migration::accept_client_migration_impl(&env, contract_id, new_client)
    }

    pub fn has_pending_client_migration(env: Env, contract_id: u32) -> bool {
        migration::has_pending_client_migration_impl(&env, contract_id)
    }

    pub fn get_pending_client_migration(env: Env, contract_id: u32) -> PendingClientMigration {
        migration::get_pending_client_migration_impl(&env, contract_id)
    }

    pub fn approve_milestone_release(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
    ) -> bool {
        if milestone_index >= MAX_MILESTONES {
            env.panic_with_error(Error::IndexOutOfBounds);
        }
        Self::require_not_paused(&env);
        Self::require_not_finalized(&env, contract_id);
        approvals::approve_milestone(&env, contract_id, milestone_index, &caller)
            .unwrap_or_else(|e| env.panic_with_error(e))
    }

    fn grant_pending_reputation_credit(env: &Env, freelancer: &Address) {
        let pending_key = DataKey::PendingReputationCredits(ReputationKey { user: freelancer.clone() });
        let pending: i128 = env.storage().persistent().get(&pending_key).unwrap_or(0);
        let new_pending = pending
            .checked_add(1)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        env.storage().persistent().set(&pending_key, &new_pending);
    }

    pub fn release_milestone(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
    ) -> bool {
        Self::require_not_paused(&env);
        caller.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if contract.status != ContractStatus::Funded {
            env.panic_with_error(Error::InvalidState);
        }

        let is_client = caller == contract.client;
        let is_freelancer = caller == contract.freelancer;
        let is_arbiter = contract.arbiter.as_ref() == Some(&caller);

        match contract.release_authorization {
            ReleaseAuthorization::ClientOnly => {
                if !is_client {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::ArbiterOnly => {
                if !is_arbiter {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::ClientAndArbiter => {
                if !is_client && !is_arbiter {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::MultiSig => {
                if !is_client && !is_freelancer {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
        }

        let milestones = ttl::load_milestones(&env, contract_id);

        if milestone_index >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let milestone = milestones.get(milestone_index).unwrap().clone();

        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }

        if milestone.refunded {
            env.panic_with_error(EscrowError::AlreadyRefunded);
        }

        approvals::check_approvals(&env, &contract, contract_id, milestone_index)
            .unwrap_or_else(|e| env.panic_with_error(e));

        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&MilestonesKey::new(contract_id))
            .unwrap();

        ttl::extend_milestone_ttl(&env, contract_id);

        if milestone_index >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let mut milestone = milestones.get(milestone_index).unwrap().clone();

        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }

        if milestone.refunded {
            env.panic_with_error(Error::AlreadyRefunded);
        }

        let available =
            contract.funded_amount - contract.released_amount - contract.refunded_amount;
        if available < milestone.amount {
            env.panic_with_error(Error::InsufficientFunds);
        }

        let gross_amount = milestone.amount;

        let protocol_fee: i128 = if Self::is_initialized(&env) {
            let fee_bps = Self::read_protocol_fee_bps(&env);
            if fee_bps > 0 {
                Self::calculate_protocol_fee(&env, gross_amount, fee_bps)
            } else {
                0
            }
        } else {
            0
        };

        let net_amount = gross_amount - protocol_fee;

        let accumulated_fees: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::AccumulatedProtocolFees)
            .unwrap_or(0);
        let available_balance = contract.funded_amount
            - contract.released_amount
            - contract.refunded_amount
            - accumulated_fees;
        if available_balance < gross_amount {
            env.panic_with_error(EscrowError::InsufficientFunds);
        }

        let token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured));
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &env.current_contract_address(),
            &contract.freelancer,
            &net_amount,
        );

        if protocol_fee > 0 {
            env.storage().persistent().set(
                &DataKey::AccumulatedProtocolFees,
                &(accumulated_fees + protocol_fee),
            );
        }

        milestone.released = true;
        milestone.funded_amount = gross_amount;
        milestones.set(milestone_index, milestone.clone());

        contract.released_amount = contract
            .released_amount
            .checked_add(net_amount)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        let new_accumulated = accumulated_fees + protocol_fee;
        let invariant_sum = contract.released_amount + contract.refunded_amount + new_accumulated;
        if invariant_sum > contract.funded_amount {
            env.panic_with_error(EscrowError::AccountingInvariantViolated);
        }

        approvals::clear_approvals(&env, contract_id, milestone_index);

        let all_released = milestones.iter().all(|m| m.released || m.refunded);
        if all_released {
            contract.status = ContractStatus::Completed;
            Self::grant_pending_reputation_credit(&env, &contract.freelancer);
        }

        ttl::store_milestones(&env, contract_id, &milestones);
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("mlstn_rls"), contract_id),
            (
                milestone_index,
                gross_amount,
                protocol_fee,
                contract.released_amount,
                caller.clone(),
                env.ledger().timestamp(),
            ),
        );

        if all_released {
            env.events().publish(
                (symbol_short!("ctrct_cmp"), contract_id),
                (caller, env.ledger().timestamp()),
            );
        }

        true
    }

    /// Releases multiple milestones in a single bounded batch call.
    ///
    /// Accepts a vector of milestone indices and releases each one sequentially,
    /// emitting a per-item `mlstn_rls` event for every successful release.
    /// The batch is capped at [`MAX_BATCH_RELEASE`] items; exceeding the cap
    /// panics with `TooManyMilestones` before any state is mutated.
    ///
    /// Per-item semantics are preserved: each milestone undergoes the full
    /// single-release validation (pause gate, auth, state checks, approval
    /// checks, funding checks, fee computation, token transfer). The caller
    /// is authenticated once at the top; individual auth checks are not
    /// repeated per item.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The escrow contract ID
    /// * `caller` - The address initiating the batch release (must be authorized)
    /// * `milestone_indices` - Vector of zero-based milestone indices to release
    ///
    /// # Returns
    /// `true` when all requested milestones were released successfully.
    ///
    /// # Errors
    /// * `TooManyMilestones` - If `milestone_indices.len() > MAX_BATCH_RELEASE`
    /// * `EmptyMilestones` - If `milestone_indices` is empty
    /// * `DuplicateMilestoneInRefund` - If duplicate indices appear in the batch
    /// * `ContractNotFound` - If `contract_id` does not exist
    /// * `ContractPaused` / `EmergencyActive` - Pause/emergency gate
    /// * `InvalidState` - If contract is not in `Funded` state
    /// * `UnauthorizedRole` - If `caller` is not authorized under the release mode
    /// * `InsufficientApprovals` - If required approvals are missing
    /// * `MilestoneAlreadyReleased` - If any target milestone is already released
    /// * `AlreadyRefunded` - If any target milestone was already refunded
    /// * `IndexOutOfBounds` - If any index exceeds the milestone count
    ///
    /// # Security
    /// - Fail-closed: the first validation failure panics the entire batch
    /// - The cap prevents unbounded computation per transaction
    /// - Duplicate detection prevents double-releases in the same batch
    ///
    /// # Events
    /// Emits one `mlstn_rls` event per successfully released milestone, and
    /// `ctrct_cmp` / `ctrct_st` if the batch completes the contract.
    pub fn release_milestones_batch(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_indices: Vec<u32>,
    ) -> bool {
        Self::require_not_paused(&env);
        caller.require_auth();

        if milestone_indices.is_empty() {
            env.panic_with_error(EscrowError::EmptyMilestones);
        }

        if milestone_indices.len() > MAX_BATCH_RELEASE {
            env.panic_with_error(EscrowError::TooManyMilestones);
        }

        // Duplicate detection
        let len = milestone_indices.len();
        for i in 0..len {
            for j in (i + 1)..len {
                if milestone_indices.get(i).unwrap() == milestone_indices.get(j).unwrap() {
                    env.panic_with_error(EscrowError::DuplicateMilestoneInRefund);
                }
            }
        }

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if contract.status != ContractStatus::Funded {
            env.panic_with_error(Error::InvalidState);
        }

        // Role authorization check (same logic as single release)
        let is_client = caller == contract.client;
        let is_freelancer = caller == contract.freelancer;
        let is_arbiter = contract.arbiter.as_ref() == Some(&caller);

        match contract.release_authorization {
            ReleaseAuthorization::ClientOnly => {
                if !is_client {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::ArbiterOnly => {
                if !is_arbiter {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::ClientAndArbiter => {
                if !is_client && !is_arbiter {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::MultiSig => {
                if !is_client && !is_freelancer {
                    env.panic_with_error(EscrowError::UnauthorizedRole);
                }
            }
        }

        let mut milestones: Vec<Milestone> = ttl::load_milestones(&env, contract_id);

        if milestone_index >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let mut milestone = milestones.get(milestone_index).unwrap().clone();

        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }

        if milestone.refunded {
            env.panic_with_error(EscrowError::AlreadyRefunded);
        }

        // Check for valid approvals
        approvals::check_approvals(&env, &contract, contract_id, milestone_index)
            .unwrap_or_else(|e| env.panic_with_error(e));

        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
            .unwrap();

        // Extend TTL on milestone read
        ttl::extend_milestone_ttl(&env, contract_id);

        if milestone_index >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let mut milestone = milestones.get(milestone_index).unwrap().clone();

        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }

        if milestone.refunded {
            env.panic_with_error(Error::AlreadyRefunded);
        }

        // Check contract-level funding (per-milestone funded_amount is set after
        // release, so we check the aggregate contract balance here).
        let available = crate::checked_available_balance(
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        )
        .unwrap_or_else(|e| env.panic_with_error(e));
        if available < milestone.amount {
            env.panic_with_error(Error::InsufficientFunds);
        }

        let gross_amount = milestone.amount;

        // Compute the protocol fee up-front so the available-balance check can
        // account for both the net payout and the fee that stays in the contract.
        //
        /// `protocol_fee` — the portion of `gross_amount` retained by the
        /// protocol. Deducted from the gross milestone amount before transfer
        /// so the escrow balance is never overdrawn.
        let protocol_fee: i128 = if Self::is_initialized(&env) {
            let fee_bps = Self::read_protocol_fee_bps(&env);
            if fee_bps > 0 {
                Self::calculate_protocol_fee(&env, gross_amount, fee_bps)
            } else {
                0
            }
        } else {
            0
        };

        /// `net_amount` — the amount actually transferred to the freelancer
        /// after deducting the protocol fee.
        let net_amount = gross_amount
            .checked_sub(protocol_fee)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));

        // The available balance must cover the full gross milestone amount
        // (net payout + fee) without dipping into already-accumulated fees or
        // other milestones' funds.
        let accumulated_fees: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::AccumulatedProtocolFees)
            .unwrap_or(0);
        let new_accumulated_fees = accumulated_fees
            .checked_add(protocol_fee)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        let available_balance = crate::checked_available_balance(
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        )
        .unwrap_or_else(|e| env.panic_with_error(e))
        .checked_sub(accumulated_fees)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::AccountingInvariantViolated));
        if available_balance < gross_amount {
            env.panic_with_error(EscrowError::InsufficientFunds);
        }

        let mut all_released_flag = false;

        // Accrue the fee into the protocol's accumulated balance.
        if protocol_fee > 0 {
            env.storage()
                .persistent()
                .set(&DataKey::AccumulatedProtocolFees, &new_accumulated_fees);
        }

        milestone.released = true;
        // Record the funded amount on the milestone so it is self-describing.
        milestone.funded_amount = gross_amount;
        milestones.set(milestone_index, milestone.clone());
        // released_amount tracks net amounts paid out to freelancers.
        // accumulated_fees tracks protocol fees retained in the contract.
        // Together: released_amount + refunded_amount + accumulated_fees <= funded_amount.
        contract.released_amount = contract
            .released_amount
            .checked_add(net_amount)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        // Accounting invariant: net released + refunded + all accumulated fees
        // must never exceed the total funded amount.
        let invariant_sum = contract
            .released_amount
            .checked_add(contract.refunded_amount)
            .and_then(|value| value.checked_add(new_accumulated_fees))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        if invariant_sum > contract.funded_amount {
            env.panic_with_error(EscrowError::AccountingInvariantViolated);
        }

        // Check if all milestones are now released or refunded
        let all_released = milestones.iter().all(|m| m.released || m.refunded);
        let old_release_status = contract.status;
        if all_released {
            contract.status = ContractStatus::Completed;
            all_released_flag = true;
            Self::grant_pending_reputation_credit(&env, &contract.freelancer);
        }

        ttl::store_milestones(&env, contract_id, &milestones);
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);
        ttl::extend_contract_ttl(&env, contract_id);

        // ── Events ──────────────────────────────────────────────────────────
        //
        // Emitted only after all state mutations succeed (fail-closed guarantee:
        // if execution reaches here, the release was accepted). Events contain
        // no secrets — all fields are already public contract state or
        // caller-supplied arguments.

        /// `mlstn_rls` — fired on every successful milestone release.
        ///
        /// Topics : `(symbol_short!("mlstn_rls"), contract_id: u32)`
        /// Data   : `(milestone_index: u32, amount: i128, fee: i128,
        ///            new_released_amount: i128, caller: Address, timestamp: u64)`
        env.events().publish(
            (symbol_short!("mlstn_rls"), contract_id),
            (
                milestone_index,
                gross_amount,
                protocol_fee,
                contract.released_amount,
                caller.clone(),
                env.ledger().timestamp(),
            ),
        );

        if protocol_fee > 0 {
            env.events().publish(
                (symbol_short!("proto_fee"), contract_id),
                (
                    milestone_index,
                    protocol_fee,
                    new_accumulated_fees,
                    env.ledger().timestamp(),
                ),
            );
        }

        // `ctrct_cmp` — fired only when this release completes the contract.
        //
        /// Topics : `(symbol_short!("ctrct_cmp"), contract_id: u32)`
        /// Data   : `(caller: Address, timestamp: u64)`
        if all_released {
            env.events().publish(
                (symbol_short!("ctrct_cmp"), contract_id),
                (caller.clone(), env.ledger().timestamp()),
            );

            env.events().publish(
                (symbol_short!("ctrct_st"), contract_id),
                (
                    old_release_status as u32,
                    ContractStatus::Completed as u32,
                    contract.funded_amount,
                    contract.released_amount,
                    contract.refunded_amount,
                    env.ledger().timestamp(),
                ),
            );
        }

        true
    }

    /// Checks if a specific milestone is overdue based on its deadline.
    ///
    /// A milestone is considered overdue if:
    /// - It has a deadline set (Some value)
    /// - The current time is strictly greater than the deadline (now > deadline)
    /// - The milestone has not been released
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `milestone_index` - The index of the milestone to check
    ///
    /// # Returns
    /// `true` if the milestone is overdue, `false` otherwise
    ///
    /// # Note
    /// - Returns `false` if milestone has no deadline (None)
    /// - Returns `false` if milestone is already released
    /// - Boundary condition: at exactly the deadline (now == deadline), returns `false`
    ///   because the deadline hasn't passed yet (uses strictly > comparison)
    ///
    /// # Security
    /// Uses `now_seconds(&env)` which is the single source of truth for ledger time.
    /// Time cannot be manipulated by contract callers.
    pub fn is_milestone_overdue(env: Env, contract_id: u32, milestone_index: u32) -> bool {
        let contract: Contract = match env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
        {
            Some(c) => c,
            None => return false,
        };

        let milestones: Vec<Milestone> = match env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
        {
            Some(m) => m,
            None => return false,
        };

        if milestone_index >= milestones.len() {
            return false;
        }

        let milestone = milestones.get(milestone_index).unwrap();

        if milestone.released {
            return false;
        }

        match milestone.deadline {
            None => false,
            Some(deadline) => now_seconds(&env) > deadline,
        }
    }

    pub fn refund_unreleased_milestones(
        env: Env,
        contract_id: u32,
        milestone_indices: Vec<u32>,
    ) -> i128 {
        Self::require_not_paused(&env);

        if milestone_indices.is_empty() {
            env.panic_with_error(EscrowError::EmptyRefundRequest);
        }

        for i in 0..milestone_indices.len() {
            for j in (i + 1)..milestone_indices.len() {
                if milestone_indices.get(i).unwrap() == milestone_indices.get(j).unwrap() {
                    env.panic_with_error(EscrowError::DuplicateMilestoneInRefund);
                }
            }
        }

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if contract.status != ContractStatus::Created
            && contract.status != ContractStatus::Funded
            && contract.status != ContractStatus::Disputed
        {
            env.panic_with_error(EscrowError::InvalidState);
        }

        contract.client.require_auth();

        let mut milestones: Vec<Milestone> = ttl::load_milestones(&env, contract_id);
        let mut total_refund_amount: i128 = 0;

        for idx in milestone_indices.iter() {
            if idx >= milestones.len() {
                env.panic_with_error(Error::IndexOutOfBounds);
            }

            let milestone = milestones.get(idx).unwrap();

            if milestone.released {
                env.panic_with_error(Error::AlreadyReleased);
            }

            if milestone.refunded {
                env.panic_with_error(EscrowError::AlreadyRefunded);
            }

            if let Some(deadline) = milestone.deadline {
                if !Self::is_milestone_overdue(env.clone(), contract_id, idx) {
                    env.panic_with_error(Error::MilestoneNotOverdue);
                }
            }

            total_refund_amount = total_refund_amount
                .checked_add(milestone.amount)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        }

        // Check if there's enough balance
        let available_balance = crate::checked_available_balance(
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        )
        .unwrap_or_else(|e| env.panic_with_error(e));
        if available_balance < total_refund_amount {
            env.panic_with_error(EscrowError::InsufficientFunds);
        }

        let token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured));

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &env.current_contract_address(),
            &contract.client,
            &total_refund_amount,
        );

        for idx in milestone_indices.iter() {
            let mut milestone = milestones.get(idx).unwrap();
            milestone.refunded = true;
            milestone.refunded_amount = milestone.amount;
            milestones.set(idx, milestone);
        }

        contract.refunded_amount = contract
            .refunded_amount
            .checked_add(total_refund_amount)
            .unwrap_or_else(|| env.panic_with_error(Error::InsufficientFunds));

        let all_refunded_or_released = milestones.iter().all(|m| m.released || m.refunded);
        let old_refund_status = contract.status;
        if all_refunded_or_released {
            let all_refunded = milestones.iter().all(|m| m.refunded);
            if all_refunded {
                contract.status = ContractStatus::Refunded;
            } else {
                contract.status = ContractStatus::Completed;
                Self::grant_pending_reputation_credit(&env, &contract.freelancer);
            }
        }

        ttl::store_milestones(&env, contract_id, &milestones);
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("refunded"), contract_id),
            (
                total_refund_amount,
                contract.status,
                env.ledger().timestamp(),
            ),
        );

        env.events().publish(
            (symbol_short!("ctrct_st"), contract_id),
            (
                old_refund_status as u32,
                contract.status as u32,
                contract.funded_amount,
                contract.released_amount,
                contract.refunded_amount,
                env.ledger().timestamp(),
            ),
        );

        total_refund_amount
    }

    pub fn contract_exists(env: Env, contract_id: u32) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::Contract(contract_id))
    }

    pub fn get_contract(env: Env, contract_id: u32) -> Contract {
        let contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        contract
    }

    pub fn get_next_contract_id(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1)
    }

    /// Returns a bounded, paginated view over created escrow contracts with
    /// compact per-entry fields.
    ///
    /// This is the contract-level counterpart to
    /// [`get_milestones_page`](Self::get_milestones_page), for indexers and
    /// UIs that need to enumerate contracts without walking the allocated ID
    /// range one `get_contract` call at a time. Each returned
    /// [`ContractEntry`] carries the contract `id`, a compact `status` code,
    /// and the `funded_amount` / `released_amount` in stroops.
    ///
    /// Contract IDs are allocated contiguously starting at `1` and are never
    /// removed from storage (cancellation, finalization, and disputes all
    /// change a contract's `status` in place), so the allocated range
    /// `[1, get_next_contract_id() - 1]` has no gaps.
    ///
    /// # Pagination contract
    ///
    /// - `start` is the zero-based offset into the sequence of created
    ///   contracts, ordered by ID (`start = 0` is contract ID `1`,
    ///   `start = 1` is contract ID `2`, and so on). An out-of-range `start`
    ///   produces an empty page (never a panic).
    /// - `limit` is clamped to `[0, PAGE_CEILING]` before use. The caller
    ///   never receives more than `PAGE_CEILING` entries per call.
    /// - Returns an empty `Vec` when no contracts have been created yet or
    ///   `start` is beyond the last created contract.
    ///
    /// # Status codes
    ///
    /// The `status` field is the [`ContractStatus`] discriminant: `0`
    /// Created, `1` Accepted, `2` Funded, `3` Completed, `4` Disputed, `5`
    /// Cancelled, `6` Refunded, `7` PartiallyFunded.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `start` - Zero-based offset of the first contract in the page
    /// * `limit` - Maximum entries to return (clamped to `PAGE_CEILING`)
    ///
    /// # Returns
    /// A [`Vec<ContractEntry>`] containing at most `min(limit, PAGE_CEILING)`
    /// entries. Empty when no contracts exist or `start` is beyond the last
    /// created contract.
    ///
    /// # Side effects
    /// Extends each returned contract's TTL, consistent with `get_contract`.
    /// Auth-free and otherwise non-mutating.
    pub fn get_contracts_page(env: Env, start: u32, limit: u32) -> Vec<ContractEntry> {
        let capped_limit = core::cmp::min(limit, PAGE_CEILING);

        let next_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1);
        // IDs are allocated contiguously starting at 1, so the number of
        // contracts ever created is `next_id - 1`.
        let total_allocated = next_id.saturating_sub(1);

        if total_allocated == 0 || start >= total_allocated {
            return Vec::new(&env);
        }

        let mut result = Vec::new(&env);
        let mut count: u32 = 0;
        let mut offset = start;
        while offset < total_allocated && count < capped_limit {
            let contract_id = offset + 1;
            let contract: Contract = env
                .storage()
                .persistent()
                .get(&DataKey::Contract(contract_id))
                .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
            ttl::extend_contract_ttl(&env, contract_id);
            result.push_back(ContractEntry {
                id: contract_id,
                status: contract.status as u32,
                funded_amount: contract.funded_amount,
                released_amount: contract.released_amount,
            });
            offset += 1;
            count += 1;
        }
        result
    }

    /// Returns a structured summary of the contract and its milestones.
    ///
    /// Extends contract and milestone TTL on read without requiring caller auth.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    ///
    /// # Returns
    /// The detailed `ContractSummary` for off-chain consumption
    ///
    /// # Errors
    /// * `ContractNotFound` - If contract doesn't exist
    pub fn get_contract_summary(env: Env, contract_id: u32) -> ContractSummary {
        let contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_and_milestones_ttl(&env, contract_id);

        let milestones = ttl::load_milestones(&env, contract_id);
        let total_amount: i128 =
            crate::amount_validation::accumulate_amounts(milestones.iter().map(|m| m.amount))
                .unwrap_or_else(|_| env.panic_with_error(EscrowError::PotentialOverflow));
        let released_milestone_count = milestones.iter().filter(|m| m.released).count() as u32;

        let mut milestone_summaries = Vec::new(&env);
        for (idx, m) in milestones.iter().enumerate() {
            milestone_summaries.push_back(MilestoneSummary {
                index: idx as u32,
                amount: m.amount,
                released: m.released,
                refunded: m.refunded,
            });
        }

        let reputation_issued = env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::ReputationIssued(contract_id))
            .unwrap_or(contract.reputation_issued);

        let refundable_balance = crate::checked_available_balance(
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        )
        .unwrap_or_else(|e| env.panic_with_error(e));

        ContractSummary {
            schema_version: CONTRACT_SUMMARY_SCHEMA_VERSION,
            client: contract.client,
            freelancer: contract.freelancer,
            arbiter: contract.arbiter,
            status: contract.status,
            reputation_issued,
            total_amount,
            funded_amount: contract.funded_amount,
            released_amount: contract.released_amount,
            refundable_balance,
            released_milestone_count,
            milestones: milestone_summaries,
        }
    }

    pub fn get_milestones(env: Env, contract_id: u32) -> Vec<Milestone> {
        let milestones = env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        ttl::extend_milestone_ttl(&env, contract_id);
        milestones
    }

    pub fn get_milestone(env: Env, contract_id: u32, milestone_index: u32) -> Option<Milestone> {
        let milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        ttl::extend_milestone_ttl(&env, contract_id);
        milestones.get(milestone_index)
    }

    /// Returns a bounded, paginated view of a contract's milestones with
    /// compact status codes.
    ///
    /// This is the read-only counterpart to [`get_milestones`](Self::get_milestones)
    /// designed for UIs that need to enumerate milestones without fetching the
    /// full vector. Each returned [`MilestoneEntry`] carries the zero-based
    /// `index`, a compact `status` code, and the milestone `amount`.
    ///
    /// # Pagination contract
    ///
    /// - `start` is the zero-based index of the first milestone to return.
    ///   An out-of-range `start` produces an empty page (never a panic).
    /// - `limit` is clamped to `[0, PAGE_CEILING]` before use. The caller
    ///   never receives more than `PAGE_CEILING` entries per call.
    /// - Returns an empty `Vec` for an unknown or empty contract rather
    ///   than panicking.
    ///
    /// # Status codes
    ///
    /// | Code | Meaning |
    /// | --- | --- |
    /// | `0` | Pending (neither released nor refunded) |
    /// | `1` | Released |
    /// | `2` | Refunded |
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `contract_id` - The escrow contract to query
    /// * `start` - Zero-based index of the first milestone in the page
    /// * `limit` - Maximum entries to return (clamped to `PAGE_CEILING`)
    ///
    /// # Returns
    /// A [`Vec<MilestoneEntry>`] containing at most `min(limit, PAGE_CEILING)`
    /// entries. Empty when the contract does not exist, has no milestones,
    /// or `start` is beyond the last milestone.
    ///
    /// # Side effects
    /// Extends the milestones vector TTL on a successful read, consistent
    /// with `get_milestones`. Auth-free and otherwise non-mutating.
    pub fn get_milestones_page(
        env: Env,
        contract_id: u32,
        start: u32,
        limit: u32,
    ) -> Vec<MilestoneEntry> {
        let capped_limit = core::cmp::min(limit, PAGE_CEILING);

        let milestones: Vec<Milestone> = match env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
        {
            Some(m) => m,
            None => return Vec::new(&env),
        };

        if milestones.is_empty() {
            return Vec::new(&env);
        }

        ttl::extend_milestone_ttl(&env, contract_id);

        let total = milestones.len();
        if start >= total {
            return Vec::new(&env);
        }

        let mut result = Vec::new(&env);
        let mut count: u32 = 0;
        let mut idx = start;
        while idx < total && count < capped_limit {
            let m = milestones.get(idx).unwrap();
            let status: u32 = if m.released {
                1
            } else if m.refunded {
                2
            } else {
                0
            };
            result.push_back(MilestoneEntry {
                index: idx,
                status,
                amount: m.amount,
            });
            idx += 1;
            count += 1;
        }
        result
    }

    /// Returns funded minus released minus refunded for `contract_id`.
    pub fn get_refundable_balance(env: Env, contract_id: u32) -> i128 {
        let contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        ttl::extend_contract_ttl(&env, contract_id);
        crate::checked_available_balance(
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        )
        .unwrap_or_else(|e| env.panic_with_error(e))
    }

    pub fn get_milestone_approvals(
        env: Env,
        contract_id: u32,
        milestone_index: u32,
    ) -> Option<MilestoneApprovals> {
        if milestone_index >= MAX_MILESTONES {
            env.panic_with_error(Error::IndexOutOfBounds);
        }
        let approval_key = crate::StorageKey::milestone_approvals(contract_id, milestone_index);
        let approvals = env.storage().temporary().get(&approval_key);
        if approvals.is_some() {
            env.storage().temporary().extend_ttl(
                &approval_key,
                ttl::PENDING_APPROVAL_BUMP_THRESHOLD,
                ttl::PENDING_APPROVAL_TTL_LEDGERS,
            );
        }
        approvals
    }

    pub fn get_approval_deadline(env: Env, contract_id: u32, milestone_index: u32) -> Option<u32> {
        if milestone_index >= MAX_MILESTONES {
            env.panic_with_error(Error::IndexOutOfBounds);
        }
        let approval_key = crate::StorageKey::milestone_approvals(contract_id, milestone_index);
        if !env.storage().temporary().has(&approval_key) {
            return None;
        }

        Some(ttl::compute_expiry(&env, ttl::PENDING_APPROVAL_TTL_LEDGERS))
    }

    pub fn pause(env: Env) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Paused, &true);

        env.events()
            .publish((symbol_short!("pause"), env.ledger().timestamp()), (admin,));
        true
    }

    pub fn unpause(env: Env) -> bool {
        Self::require_initialized(&env);
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Emergency)
            .unwrap_or(false)
        {
            env.panic_with_error(Error::EmergencyActive);
        }
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Paused, &false);

        env.events().publish(
            (symbol_short!("unpaused"), env.ledger().timestamp()),
            (admin,),
        );
        true
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn activate_emergency_pause(env: Env) -> bool {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(Error::NotInitialized));

        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            admin.require_auth();
        }
        env.storage().persistent().set(&DataKey::Emergency, &true);
        env.storage().persistent().set(&DataKey::Paused, &true);

        let mut checklist: ReadinessChecklist = env
            .storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default();
        checklist.emergency_controls_enabled = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);

        env.events().publish(
            (
                Symbol::new(&env, "emergency"),
                Symbol::new(&env, "activated"),
            ),
            (
                env.storage()
                    .persistent()
                    .get::<_, Address>(&DataKey::Admin)
                    .unwrap(),
                env.ledger().timestamp(),
            ),
        );
        true
    }

    pub fn resolve_emergency(env: Env) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(Error::NotInitialized));
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Emergency, &false);
        env.storage().persistent().set(&DataKey::Paused, &false);

        let mut checklist: ReadinessChecklist = env
            .storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default();
        checklist.emergency_controls_enabled = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);
        env.events().publish(
            (
                Symbol::new(&env, "emergency"),
                Symbol::new(&env, "resolved"),
            ),
            (admin, env.ledger().timestamp()),
        );
        true
    }

    pub fn is_emergency(env: Env) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Emergency)
            .unwrap_or(false)
    }

    pub fn set_protocol_fee_bps(env: Env, new_bps: u32) -> bool {
        Self::set_protocol_fee_bps_impl(env, new_bps)
    }

    pub fn get_governance_admin(env: Env) -> Option<Address> {
        Self::get_governance_admin_impl(env)
    }

    pub fn get_protocol_fee_bps(env: Env) -> u32 {
        Self::get_protocol_fee_bps_impl(env)
    }

    pub fn propose_governance_admin(env: Env, proposed: Address) -> bool {
        Self::propose_governance_admin_impl(&env, proposed)
    }

    pub fn accept_governance_admin(env: Env) -> bool {
        Self::accept_governance_admin_impl(&env)
    }

    pub fn set_governed_params(
        env: Env,
        admin: Address,
        protocol_fee_bps: u32,
        max_escrow_total_stroops: i128,
    ) -> bool {
        Self::set_governed_params_impl(
            env,
            admin,
            protocol_fee_bps,
            max_escrow_total_stroops,
        )
    }

    pub fn get_governed_parameters(env: Env) -> Option<GovernedParameters> {
        Self::get_governed_parameters_impl(env)
    }

    // ── Cancel contract ──────────────────────────────────────────────────────

    pub fn cancel_contract(env: Env, contract_id: u32, caller: Address) -> bool {
        Self::require_not_paused(&env);
        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        ttl::extend_contract_ttl(&env, contract_id);

        Self::require_not_finalized(&env, contract_id);

        if caller != contract.client {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }

        if contract.status == ContractStatus::Cancelled {
            env.panic_with_error(Error::AlreadyCancelled);
        }

        if contract.status != ContractStatus::Created && contract.status != ContractStatus::Funded {
            env.panic_with_error(EscrowError::InvalidStatusTransition);
        }

        if contract.released_amount != 0 {
            env.panic_with_error(EscrowError::InvalidStatusTransition);
        }

        caller.require_auth();

        let refund_amount = crate::checked_available_balance(
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        )
        .unwrap_or_else(|e| env.panic_with_error(e));

        if refund_amount > 0 {
            let token = Self::read_settlement_token(&env)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &caller,
                &refund_amount,
            );
        }

        let old_status = contract.status;
        contract.status = ContractStatus::Cancelled;
        contract.refunded_amount = safe_add_amounts(contract.refunded_amount, refund_amount)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);
        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("cancelled"), contract_id),
            (caller, refund_amount, env.ledger().timestamp()),
        );

        env.events().publish(
            (symbol_short!("ctrct_st"), contract_id),
            (
                old_status as u32,
                ContractStatus::Cancelled as u32,
                contract.funded_amount,
                contract.released_amount,
                contract.refunded_amount,
                env.ledger().timestamp(),
            ),
        );

        true
    }

    fn load_checklist(env: &Env) -> ReadinessChecklist {
        env.storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default()
    }

    // ─── Configurable limits ──────────────────────────────────────────────────

    /// Returns the effective max milestones, falling back to the default.
    fn effective_max_milestones(env: &Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MaxMilestones)
            .unwrap_or(DEFAULT_MAX_MILESTONES)
    }

    /// Returns the effective max escrow stroops, falling back to the default.
    fn effective_max_escrow_stroops(env: &Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::MaxEscrowStroops)
            .unwrap_or(DEFAULT_MAX_TOTAL_ESCROW_STROOPS)
    }

    /// Set the max milestones limit. Admin only. Rejects out-of-range values.
    pub fn set_max_milestones(env: Env, max_milestones: u32) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();

        if max_milestones < MIN_MAX_MILESTONES || max_milestones > MAX_MAX_MILESTONES {
            env.panic_with_error(EscrowError::LimitOutOfRange);
        }

        env.storage()
            .persistent()
            .set(&DataKey::MaxMilestones, &max_milestones);

        env.events().publish(
            (symbol_short!("limits"), Symbol::new(&env, "max_milestones")),
            (max_milestones, env.ledger().timestamp()),
        );
        true
    }

    /// Returns the current max milestones limit (or the default if not set).
    pub fn get_max_milestones(env: Env) -> u32 {
        Self::effective_max_milestones(&env)
    }

    /// Set the max escrow stroops limit. Admin only. Rejects out-of-range values.
    pub fn set_max_escrow_stroops(env: Env, max_escrow_stroops: i128) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();

        if max_escrow_stroops < MIN_MAX_ESCROW_STROOPS
            || max_escrow_stroops > MAINNET_MAX_TOTAL_ESCROW_PER_CONTRACT_STROOPS
        {
            env.panic_with_error(EscrowError::LimitOutOfRange);
        }

        env.storage()
            .persistent()
            .set(&DataKey::MaxEscrowStroops, &max_escrow_stroops);

        env.events().publish(
            (symbol_short!("limits"), Symbol::new(&env, "max_escrow")),
            (max_escrow_stroops, env.ledger().timestamp()),
        );
        true
    }

    /// Returns the current max escrow stroops limit (or the default if not set).
    pub fn get_max_escrow_stroops(env: Env) -> i128 {
        Self::effective_max_escrow_stroops(&env)
    }

    // ─── Contract lifecycle ───────────────────────────────────────────────────



    pub fn issue_reputation(
        env: Env,
        contract_id: u32,
        caller: Address,
        rating: u32,
        comment: String,
    ) -> bool {
        Self::require_not_paused(&env);
        Self::validate_contract_id_bounds(&env, contract_id);
        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
        ttl::extend_contract_ttl(&env, contract_id);

        if caller != contract.client {
            env.panic_with_error(Error::UnauthorizedRole);
        }

        if rating < 1 || rating > 5 {
            env.panic_with_error(Error::InvalidRating);
        }

        if comment.len() == 0 {
            env.panic_with_error(Error::EmptyComment);
        }

        if comment.len() > 200 {
            env.panic_with_error(Error::CommentTooLong);
        }

        if contract.status != ContractStatus::Completed {
            env.panic_with_error(Error::NotCompleted);
        }

        if contract.reputation_issued {
            env.panic_with_error(Error::ReputationAlreadyIssued);
        }
        if contract.client == contract.freelancer {
            env.panic_with_error(Error::SelfRating);
        }

        caller.require_auth();
        contract.reputation_issued = true;
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);
        env.storage()
            .persistent()
            .set(&DataKey::ReputationIssued(contract_id), &true);
        env.storage().persistent().extend_ttl(
            &DataKey::ReputationIssued(contract_id),
            ttl::PERSISTENT_BUMP_THRESHOLD,
            ttl::PERSISTENT_TTL_LEDGERS,
        );

        let pending_key = DataKey::PendingReputationCredits(ReputationKey { user: contract.freelancer.clone() });
        let pending: i128 = env.storage().persistent().get(&pending_key).unwrap_or(0);
        if pending <= 0 {
            env.panic_with_error(Error::InvalidState);
        }
        env.storage().persistent().set(&pending_key, &(pending - 1));

        let rep_key = DataKey::Reputation(ReputationKey { user: contract.freelancer.clone() });
        let mut rep: types::Reputation =
            env.storage().persistent().get(&rep_key).unwrap_or_default();
        rep.completed_contracts = rep
            .completed_contracts
            .checked_add(1)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
        rep.total_rating = rep
            .total_rating
            .checked_add(rating as i128)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
        rep.last_rating = rating as i128;
        env.storage().persistent().set(&rep_key, &rep);

        let comment_key = DataKey::ReputationComment(contract_id);
        env.storage().persistent().set(&comment_key, &comment);
        env.storage().persistent().extend_ttl(
            &comment_key,
            ttl::PERSISTENT_BUMP_THRESHOLD,
            ttl::PERSISTENT_TTL_LEDGERS,
        );

        // ── Events ──────────────────────────────────────────────────────────
        //
        // Emitted only after every storage mutation has succeeded (fail-closed
        // guarantee: a panic in any earlier check prevents this publish, so the
        // event observes only fully-applied reputation state).
        //
        /// `rep_issue` — fired on every successful reputation issuance so
        /// off-chain indexers can cheaply reconstruct the full reputation
        /// history of every freelancer without re-fetching contract storage
        /// after each individual `issue_reputation` call.
        ///
        /// Topics : `(symbol_short!("rep_issue"), contract_id: u32)`
        ///   - `rep_issue` is a `symbol_short!` 9-char ASCII string, fitting
        ///     within the Soroban compile-time short-symbol length check.
        ///   - The topic does not collide with any other event topic in this
        ///     contract (`init`, `mlstn_rls`, `ctrct_cmp`, `refunded`,
        ///     `pause`, `unpaused`, `cancelled`, `evidence`, `fee`,
        ///     `dispute`), giving indexers an unambiguous per-action filter.
        ///   - The second topic element is `contract_id`, matching the
        ///     per-contract scoping used by `mlstn_rls`, `ctrct_cmp`,
        ///     `refunded`, `cancelled`, and `evidence`. This lets an indexer
        ///     subscribe to a single contract's reputation stream, and —
        ///     since each contract can only call `issue_reputation` once —
        ///     the topic guarantees at-most-one event per contract_id.
        ///   - Indexers that want a per-freelancer feed can filter on
        ///     `freelancer` in the data payload instead.
        ///
        /// Data   : `(client: Address, freelancer: Address, rating: u32,
        ///            total_rating: i128, completed_contracts: i128,
        ///            timestamp: u64)`
        ///   - `client`: the rater (must equal the stored `contract.client`,
        ///     an invariant enforced by the caller-auth check above).
        ///   - `freelancer`: the reputation subject; indexable as the
        ///     primary key for a per-freelancer reputation feed.
        ///   - `rating`: the per-issuance rating value (1..=5).
        ///   - `total_rating`: the cumulative rating sum after this
        ///     issuance, so the indexer can compute running averages
        ///     without an extra storage read.
        ///   - `completed_contracts`: cumulative count of completed
        ///     contracts after this issuance, paired with `total_rating`
        ///     for the same reason.
        ///   - `timestamp`: ledger timestamp at issuance.
        env.events().publish(
            (symbol_short!("rep_issue"), contract_id),
            (
                contract.client.clone(),
                contract.freelancer.clone(),
                rating,
                rep.total_rating,
                rep.completed_contracts,
                env.ledger().timestamp(),
            ),
        );

        true
    }

    pub fn issue_reputation_batch(
        env: Env,
        caller: Address,
        items: Vec<ReputationBatchItem>,
    ) -> bool {
        Self::require_not_paused(&env);
        if items.len() > MAX_REPUTATION_BATCH_SIZE {
            env.panic_with_error(Error::BatchItemLimitExceeded);
        }
        caller.require_auth();
        let mut i = 0;
        while i < items.len() {
            let item = items.get(i).unwrap();
            Self::validate_contract_id_bounds(&env, item.contract_id);
            let mut contract: Contract = env
                .storage()
                .persistent()
                .get(&DataKey::Contract(item.contract_id))
                .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
            ttl::extend_contract_ttl(&env, item.contract_id);
            if caller != contract.client {
                env.panic_with_error(Error::UnauthorizedRole);
            }
            if item.rating < 1 || item.rating > 5 {
                env.panic_with_error(Error::InvalidRating);
            }
            if item.comment.len() == 0 {
                env.panic_with_error(Error::EmptyComment);
            }
            if item.comment.len() > 200 {
                env.panic_with_error(Error::CommentTooLong);
            }
            if contract.status != ContractStatus::Completed {
                env.panic_with_error(Error::NotCompleted);
            }
            if contract.reputation_issued {
                env.panic_with_error(Error::ReputationAlreadyIssued);
            }
            if contract.client == contract.freelancer {
                env.panic_with_error(Error::SelfRating);
            }
            contract.reputation_issued = true;
            env.storage()
                .persistent()
                .set(&DataKey::Contract(item.contract_id), &contract);
            env.storage()
                .persistent()
                .set(&DataKey::ReputationIssued(item.contract_id), &true);
            env.storage().persistent().extend_ttl(
                &DataKey::ReputationIssued(item.contract_id),
                ttl::PERSISTENT_BUMP_THRESHOLD,
                ttl::PERSISTENT_TTL_LEDGERS,
            );
            let pending_key = DataKey::PendingReputationCredits(contract.freelancer.clone());
            let pending: i128 = env.storage().persistent().get(&pending_key).unwrap_or(0);
            if pending <= 0 {
                env.panic_with_error(Error::InvalidState);
            }
            env.storage().persistent().set(&pending_key, &(pending - 1));
            let rep_key = DataKey::Reputation(contract.freelancer.clone());
            let mut rep: types::Reputation =
                env.storage().persistent().get(&rep_key).unwrap_or_default();
            rep.completed_contracts = rep
                .completed_contracts
                .checked_add(1)
                .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
            rep.total_rating = rep
                .total_rating
                .checked_add(item.rating as i128)
                .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
            rep.last_rating = item.rating as i128;
            env.storage().persistent().set(&rep_key, &rep);
            let comment_key = DataKey::ReputationComment(item.contract_id);
            env.storage().persistent().set(&comment_key, &item.comment);
            env.storage().persistent().extend_ttl(
                &comment_key,
                ttl::PERSISTENT_BUMP_THRESHOLD,
                ttl::PERSISTENT_TTL_LEDGERS,
            );
            env.events().publish(
                (symbol_short!("rep_iss"), item.contract_id),
                (caller.clone(), item.rating, env.ledger().timestamp()),
            );
            i += 1;
        }
        true
    }

    pub fn get_reputation_comment(env: Env, contract_id: u32) -> Option<String> {
        Self::validate_contract_id_bounds(&env, contract_id);
        let comment_key = DataKey::ReputationComment(contract_id);
        let comment: Option<String> = env.storage().persistent().get(&comment_key);
        if comment.is_some() {
            env.storage().persistent().extend_ttl(
                &comment_key,
                ttl::PERSISTENT_BUMP_THRESHOLD,
                ttl::PERSISTENT_TTL_LEDGERS,
            );
        }
        comment
    }

    pub fn get_reputation(env: Env, address: Address) -> Option<types::Reputation> {
        env.storage()
            .persistent()
            .get(&DataKey::Reputation(ReputationKey { user: address }))
    }

    pub fn get_average_rating(env: Env, address: Address) -> Option<i128> {
        const SCALE: i128 = 10_000;

        let rep: types::Reputation = env
            .storage()
            .persistent()
            .get(&DataKey::Reputation(ReputationKey { user: address }))?;

        if rep.completed_contracts == 0 {
            return None;
        }

        rep.total_rating
            .checked_mul(SCALE)
            .and_then(|scaled| scaled.checked_div(rep.completed_contracts))
    }

    pub fn get_pending_reputation_credits(env: Env, address: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingReputationCredits(ReputationKey { user: address }))
            .unwrap_or(0)
    }

    pub fn submit_work_evidence(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
        evidence: String,
    ) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        Self::validate_contract_id_bounds(&env, contract_id);
        caller.require_auth();

        let contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if caller != contract.freelancer {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }

        if contract.status != ContractStatus::Funded {
            env.panic_with_error(EscrowError::InvalidState);
        }

        if evidence.len() > 256 {
            env.panic_with_error(Error::EvidenceTooLong);
        }

        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_milestone_ttl(&env, contract_id);

        if milestone_index >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let mut milestone = milestones.get(milestone_index).unwrap();

        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }
        if milestone.refunded {
            env.panic_with_error(EscrowError::AlreadyRefunded);
        }

        milestone.work_evidence = Some(evidence.clone());
        milestones.set(milestone_index, milestone);

        ttl::store_milestones(&env, contract_id, &milestones);

        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("evidence"), contract_id),
            (
                milestone_index,
                contract.freelancer,
                env.ledger().timestamp(),
            ),
        );

        true
    }

    pub fn get_work_evidence(env: Env, contract_id: u32, milestone_index: u32) -> Option<String> {
        Self::validate_contract_id_bounds(&env, contract_id);
        let milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&crate::StorageKey::contract_milestones(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_milestone_ttl(&env, contract_id);

        if milestone_index >= milestones.len() {
            return None;
        }

        milestones.get(milestone_index).unwrap().work_evidence
    }

    pub fn get_accumulated_protocol_fees(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get::<_, i128>(&DataKey::AccumulatedProtocolFees)
            .unwrap_or(0)
    }

    pub fn withdraw_protocol_fees(env: Env, amount: i128, to: Address) -> bool {
        Self::require_initialized(&env);

        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::ContractPaused);
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));

        admin.require_auth();

        if amount <= 0 {
            env.panic_with_error(EscrowError::AmountMustBePositive);
        }

        let accumulated: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::AccumulatedProtocolFees)
            .unwrap_or(0);

        if amount > accumulated {
            env.panic_with_error(EscrowError::InsufficientAccumulatedFees);
        }

        let token_client = Self::require_settlement_token(&env);

        let new_accumulated = accumulated
            .checked_sub(amount)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::InsufficientAccumulatedFees));
        env.storage()
            .persistent()
            .set(&DataKey::AccumulatedProtocolFees, &new_accumulated);

        env.storage().persistent().extend_ttl(
            &DataKey::AccumulatedProtocolFees,
            ttl::PERSISTENT_BUMP_THRESHOLD,
            ttl::PERSISTENT_TTL_LEDGERS,
        );

        token_client.transfer(&env.current_contract_address(), &to, &amount);

        env.events().publish(
            (symbol_short!("fee"), symbol_short!("withdraw")),
            (admin, to, amount, env.ledger().timestamp()),
        );

        true
    }

    pub fn get_pending_admin_proposed_at(env: Env) -> Option<u32> {
        let proposal: Option<PendingAdminProposal> =
            env.storage().persistent().get(&DataKey::PendingAdmin);
        proposal.map(|p| p.proposed_at_ledger)
    }

    pub(crate) fn read_protocol_fee_bps(env: &Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ProtocolFeeBps)
            .unwrap_or(0)
    }

    pub fn calculate_protocol_fee(env: &Env, amount: i128, fee_bps: u32) -> i128 {
        if fee_bps == 0 {
            return 0;
        }
        let product = amount
            .checked_mul(fee_bps as i128)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
        product / BASIS_POINT_DENOMINATOR as i128
    }

    // ── Internal guards ──────────────────────────────────────────────────────

    /// Panics with `NotInitialized` unless `initialize` has been called.
    pub(crate) fn require_initialized(env: &Env) {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(Error::NotInitialized);
        }
    }

    fn is_initialized(env: &Env) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
    }

    /// Defensive bounds check: `contract_id` must fall within the allocated
    /// range `[1, get_next_contract_id() - 1]`.
    ///
    /// Contract IDs are allocated contiguously starting at `1` and are never
    /// removed from storage (see [`Escrow::get_contracts_page`]), so any ID
    /// outside this range is guaranteed to be unallocated. This is a cheap
    /// pre-check ahead of a storage lookup; it does not change the security
    /// model — an out-of-range ID would fail the subsequent storage `.get`
    /// with the same `ContractNotFound` error regardless.
    pub(crate) fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
        let next_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1);
        if contract_id == 0 || contract_id >= next_id {
            env.panic_with_error(EscrowError::ContractNotFound);
        }
    }

    // -----------------------------------------------------------------------
    // Dispute management
    // -----------------------------------------------------------------------

    /// Opens a dispute for a funded or partially funded escrow contract.
    ///
    /// This entrypoint transitions the contract status to `Disputed`, preventing
    /// further milestone releases until an assigned arbiter resolves the dispute.
    /// Only the client or freelancer can open a dispute, and an arbiter must be
    /// assigned to the contract.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `caller` - The address opening the dispute (must be client or freelancer)
    ///
    /// # Returns
    /// `true` if the dispute was successfully opened
    ///
    /// # Errors
    /// * `NotInitialized` - If `initialize` has not been called
    /// * `ContractNotFound` - If contract doesn't exist
    /// * `UnauthorizedRole` - If caller is not client or freelancer
    /// * `ArbiterRequired` - If no arbiter is assigned to the contract
    /// * `InvalidState` - If contract is not in a disputable state
    /// * `ContractPaused` - If pause or emergency controls are active
    /// * `AlreadyFinalized` - If contract has been finalized
    ///
    /// # Security
    /// - Only contract parties (client/freelancer) can open disputes
    /// - Requires arbiter assignment for resolution
    /// - Blocks milestone releases while disputed
    /// - Respects pause and emergency controls
    pub fn raise_dispute(env: Env, contract_id: u32, caller: Address) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        Self::validate_contract_id_bounds(&env, contract_id);
        caller.require_auth();
        Self::raise_dispute_inner(&env, contract_id, &caller);
        true
    }

    /// Shared raise-dispute mutation used by the single and batch entrypoints.
    ///
    /// Callers must already have completed `require_initialized`, `require_not_paused`,
    /// and `caller.require_auth()` so batch invocations authenticate once.
    fn raise_dispute_inner(env: &Env, contract_id: u32, caller: &Address) {
        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(env, contract_id);
        Self::require_not_finalized(env, contract_id);

        if caller != contract.client && caller != contract.freelancer {
            env.panic_with_error(Error::UnauthorizedRole);
        }

        if contract.arbiter.is_none() {
            env.panic_with_error(Error::ArbiterRequired);
        }

        match contract.status {
            ContractStatus::Funded | ContractStatus::PartiallyFunded => {}
            _ => env.panic_with_error(Error::InvalidState),
        }

        let old_status = contract.status;
        contract.status = ContractStatus::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_ttl(env, contract_id);

        env.events().publish(
            (symbol_short!("dispute"), symbol_short!("opened")),
            (contract_id, caller.clone()),
        );

        env.events().publish(
            (symbol_short!("ctrct_st"), contract_id),
            (
                old_status as u32,
                ContractStatus::Disputed as u32,
                contract.funded_amount,
                contract.released_amount,
                contract.refunded_amount,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Batch variant of [`raise_dispute`](Self::raise_dispute) that accepts a
    /// bounded vector of contract IDs.
    ///
    /// If the vector length exceeds [`MAX_BATCH_DISPUTES`], the call is rejected
    /// with [`EscrowError::BatchCapExceeded`]. Per-item semantics are preserved:
    /// each contract ID goes through the same authorization and state checks as
    /// the single entrypoint, and a `("dispute", "opened")` event is emitted per
    /// successfully disputed contract. On any per-item failure the whole call
    /// panics and the transaction rolls back (all-or-nothing).
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `caller` - The address of the caller (must be a party on every contract)
    /// * `contract_ids` - Bounded vector of contract IDs to dispute
    ///
    /// # Errors
    /// * `BatchCapExceeded` - If `contract_ids` length exceeds the cap
    /// * All errors from [`raise_dispute`](Self::raise_dispute)
    ///
    /// # Events
    /// Emits `("dispute", "opened")` with payload `(contract_id, caller)` for
    /// each successfully opened dispute.
    pub fn raise_dispute_batch(env: Env, caller: Address, contract_ids: Vec<u32>) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        caller.require_auth();

        if contract_ids.len() > MAX_BATCH_DISPUTES {
            env.panic_with_error(EscrowError::BatchCapExceeded);
        }

        let mut i: u32 = 0;
        while i < contract_ids.len() {
            let contract_id = contract_ids.get(i).unwrap();
            Self::raise_dispute_inner(&env, contract_id, &caller);
            i += 1;
        }

        true
    }

    pub fn resolve_dispute(
        env: Env,
        contract_id: u32,
        arbiter: Address,
        resolution: DisputeResolution,
    ) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        Self::validate_contract_id_bounds(&env, contract_id);
        arbiter.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if contract.status != ContractStatus::Disputed {
            env.panic_with_error(Error::InvalidStatusTransition);
        }

        match &contract.arbiter {
            Some(contract_arbiter) if *contract_arbiter == arbiter => {}
            _ => env.panic_with_error(Error::UnauthorizedRole),
        }

        let (client_payout, freelancer_payout) =
            dispute::resolution_payouts(&contract, &resolution)
                .unwrap_or_else(|e| env.panic_with_error(e));

        // Update contract accounting — use checked arithmetic to guard against
        // overflow at extreme values (Issue #890).
        contract.refunded_amount = contract
            .refunded_amount
            .checked_add(client_payout)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        contract.released_amount = contract
            .released_amount
            .checked_add(freelancer_payout)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        // Set final status
        let final_status = dispute::final_status_after_resolution(&contract);
        let old_status = contract.status;
        contract.status = final_status;
        if contract.status == ContractStatus::Completed {
            Self::grant_pending_reputation_credit(&env, &contract.freelancer);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("dispute"), symbol_short!("resolved")),
            (contract_id, resolution.code()),
        );

        env.events().publish(
            (symbol_short!("ctrct_st"), contract_id),
            (
                old_status as u32,
                contract.status as u32,
                contract.funded_amount,
                contract.released_amount,
                contract.refunded_amount,
                env.ledger().timestamp(),
            ),
        );

        true
    }
}

#[cfg(test)]
mod test;
