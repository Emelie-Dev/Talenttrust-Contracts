//! TalentTrust escrow contract for milestone-based freelancer payments.
//!
//! The crate root exposes the Soroban contract and still owns several public
//! entrypoints directly: initialization, settlement-token binding, deposits,
//! milestone release/refund/cancel flows, reputation (including batch), work
//! evidence, protocol fee withdrawal, and dispute entrypoints. Supporting modules keep reusable
//! validation, storage, governance, and lifecycle helpers close to the paths
//! that use them.
//!
//! ## Escrow source tree map
//!
//! | Source | Responsibility | Storage keys owned or touched |
//! | --- | --- | --- |
//! | `lib.rs` | Contract wrapper plus root entrypoints for setup, custody, money movement, reads, reputation (incl. batch), work evidence, pause/emergency, fee withdrawal, and dispute orchestration. | `DataKey::Initialized`, `Admin`, `SettlementToken`, `Paused`, `Emergency`, `ReadinessChecklist`, `Contract(id)`, `(Contract(id), "milestones")`, `MilestoneApprovals`, `AccumulatedProtocolFees`, `ReputationIssued`, `PendingReputationCredits`, `Reputation`, `ReputationComment` |
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
// Keep shared storage keys and escrow domain types centralized in `types.rs`.
// `DisputeResolution` and `DisputeSplit` are defined once in `types.rs` and
// re-exported here; `dispute.rs` uses them via `crate::DisputeResolution`.
pub use types::{
    Contract, ContractBounds, ContractStatus, ContractSummary, DataKey, DepositMode,
    DisputeResolution, DisputeSplit, Error, GovernedParameters, Milestone, MilestoneApprovals,
    MilestoneEntry, MilestoneSummary, PendingAdminProposal, ReadinessChecklist,
    ReleaseAuthorization, Reputation, ReputationBatchItem, SplitAmounts, CONTRACT_SUMMARY_SCHEMA_VERSION,
};

/// Default maximum number of milestones allowed per contract.
pub const DEFAULT_MAX_MILESTONES: u32 = 10;

/// Default hard cap on the total escrow value per contract, in stroops.
pub const DEFAULT_MAX_TOTAL_ESCROW_STROOPS: i128 = 10_000_000_000_000;

/// Backward-compatible alias for the default max milestones.
pub const MAX_MILESTONES: u32 = DEFAULT_MAX_MILESTONES;

/// Backward-compatible alias for the default max escrow stroops.
pub const MAX_TOTAL_ESCROW_STROOPS: i128 = DEFAULT_MAX_TOTAL_ESCROW_STROOPS;

/// Absolute minimum for the max milestones setting.
pub const MIN_MAX_MILESTONES: u32 = 1;

/// Absolute maximum for the max milestones setting.
pub const MAX_MAX_MILESTONES: u32 = 100;

/// Absolute minimum for the max escrow stroops setting (0.01 XLM).
pub const MIN_MAX_ESCROW_STROOPS: i128 = 1_000_000;

pub const MAINNET_PROTOCOL_VERSION: u32 = 1u32;
pub const MAINNET_MAX_TOTAL_ESCROW_PER_CONTRACT_STROOPS: i128 = 1_000_000_000_000_000i128;

/// Maximum number of reputation items accepted in a single batch call.
pub const MAX_REPUTATION_BATCH_SIZE: usize = 10;

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
pub struct MilestoneApprovals {
    pub client_approved: bool,
    pub freelancer_approved: bool,
    pub arbiter_approved: bool,
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
}

type Error = EscrowError;

impl Escrow {
    /// Get the settlement token address from the canonical `DataKey` binding.
    pub(crate) fn read_settlement_token(env: &Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::SettlementToken)
    }

    /// Persist the settlement token address under the canonical `DataKey` binding.
    pub(crate) fn write_settlement_token(env: &Env, token: &Address) {
        env.storage()
            .persistent()
            .set(&DataKey::SettlementToken, token);
    }
}

#[contractimpl]
impl Escrow {
    /// Bind the single Stellar Asset Contract (SAC) token this escrow instance will custody.
    ///
    /// This is a **write-once** step: once a token is recorded under
    /// [`DataKey::SettlementToken`] all subsequent money-flow entrypoints
    /// (`deposit_funds`, `release_milestone`, `refund_unreleased_milestones`,
    /// `cancel_contract`, `withdraw_protocol_fees`) read that address to execute SAC
    /// `transfer` calls.  A second call with any token address is rejected with
    /// `SettlementTokenAlreadyBound`.
    ///
    /// # Pre-bind probe (issue #723)
    ///
    /// Before persisting the token address, this entrypoint performs a **read-only
    /// probe** to verify the supplied address is a live SAC token contract:
    ///
    /// 1. Calls `token::Client::balance(env.current_contract_address())` against
    ///    the candidate address. If the address does not implement the SAC token
    ///    interface, the call panics and the bind is rejected with
    ///    `InvalidSettlementToken`.
    /// 2. Rejects `env.current_contract_address()` (the escrow contract itself)
    ///    with `SettlementTokenIsSelf` — binding self creates a circular custody
    ///    reference.
    /// 3. Rejects the stored admin address with `SettlementTokenIsAdmin` —
    ///    conflating governance authority with the settlement token role is a
    ///    privilege-separation violation.
    ///
    /// # Reentrancy mitigation
    ///
    /// All downstream money-flow entrypoints (`deposit_funds`, `release_milestone`,
    /// `cancel_contract`, `refund_unreleased_milestones`) follow strict
    /// **state-before-transfer** (Checks-Effects-Interactions) ordering: contract
    /// state is finalized *before* any `token::Client::transfer` call.  A
    /// malicious token contract that re-enters the escrow during a transfer will
    /// observe the already-mutated state and cannot double-spend or front-run
    /// the operation.  The probe itself performs no state mutation — it only
    /// reads the token balance — so it cannot be used as a reentrancy vector.
    ///
    /// See [`docs/escrow/sac-custody.md`](../../../docs/escrow/sac-custody.md) for the
    /// full custody model, accounting invariant, and lifecycle sequence diagram.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `admin` - The admin address (must match stored admin)
    /// * `token` - The SAC token address
    ///
    /// # Errors
    /// * `NotInitialized` if `initialize` has not been called
    /// * `UnauthorizedRole` if `admin` is not the stored admin
    /// * `SettlementTokenAlreadyBound` if a token is already bound
    /// * `InvalidSettlementToken` if the probe call to `token::Client::balance` panics
    /// * `SettlementTokenIsSelf` if `token == env.current_contract_address()`
    /// * `SettlementTokenIsAdmin` if `token == stored_admin`
    ///
    /// # Events
    /// On a successful, authorized bind this publishes a `settlement_token_bound`
    /// event so off-chain indexers and monitoring dashboards can observe which
    /// asset an escrow settles in, and when the binding happened.
    ///
    /// * Topics: `(Symbol "settlement_token_bound",)`
    /// * Data: `(admin: Address, token: Address, timestamp: u64)`
    ///
    /// The event only fires after the write succeeds. Rejected binds
    /// (uninitialized, unauthorized, invalid token, self, admin) panic before
    /// this point and therefore publish nothing. All payload fields are public
    /// configuration.
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

        // Reject double-bind: once a settlement token is recorded, any
        // subsequent bind attempt is rejected. This is a write-once field.
        if Self::read_settlement_token(&env).is_some() {
            env.panic_with_error(EscrowError::SettlementTokenAlreadyBound);
        }

        // ── Pre-bind probe (issue #723) ─────────────────────────────────────
        //
        // Reject the escrow contract's own address — binding self would create
        // a circular custody reference and brick every transfer path.
        if token == env.current_contract_address() {
            env.panic_with_error(EscrowError::SettlementTokenIsSelf);
        }

        // Reject the admin address — conflating governance authority with the
        // settlement token role is a privilege-separation violation.
        if token == stored_admin {
            env.panic_with_error(EscrowError::SettlementTokenIsAdmin);
        }

        // Read-only probe: call `token::Client::balance` against the escrow
        // contract address. If `token` does not implement the SAC token
        // interface, the host panics and we translate that into
        /// `InvalidSettlementToken`.
        //
        // This is safe because:
        // - `balance` is a read-only entrypoint (no state mutation on the
        //   token contract).
        // - We have not yet written anything to storage — a panic here leaves
        //   no partial state.
        // - The probe cannot be used for reentrancy: it calls `balance`, not
        //   `transfer`, and the escrow has no callback the token could invoke.
        let token_client = token::Client::new(&env, &token);
        let _probe: i128 = token_client.balance(&env.current_contract_address());

        Self::write_settlement_token(&env, &token);

        // Emit after the binding write succeeds so indexers can track the bound
        // asset. Consistent topic naming with `init` / `protocol_fee_bps` events.
        env.events().publish(
            (Symbol::new(&env, "settlement_token_bound"),),
            (admin, token, env.ledger().timestamp()),
        );
        true
    }

    /// Deprecated thin delegate for [`bind_settlement_token`](Self::bind_settlement_token).
    ///
    /// Retained for backward compatibility with external callers that used the historical API name.
    /// Delegates directly to [`bind_settlement_token`](Self::bind_settlement_token) and inherits
    /// every security guard (`SettlementTokenAlreadyBound`, admin auth check, SAC interface probe,
    /// self/admin validation) and event emission.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `admin` - The admin address (must match stored admin)
    /// * `token` - The SAC token address
    ///
    /// # Deprecated
    /// Use [`bind_settlement_token`](Self::bind_settlement_token) instead.
    #[deprecated(note = "Use bind_settlement_token instead.")]
    pub fn set_settlement_token(env: Env, admin: Address, token: Address) -> bool {
        Self::bind_settlement_token(env, admin, token)
    }

    /// Returns the bound settlement token, or `None` if no token has been bound.
    pub fn get_settlement_token(env: Env) -> Option<Address> {
        Self::read_settlement_token(&env)
    }

    /// Returns `true` exactly when a settlement token is bound.
    ///
    /// This is the recommended cheap pre-flight readiness check before calling
    /// `deposit_funds`, which panics when no settlement token has been bound.
    /// Integrators that only need to know *whether* the escrow can accept
    /// deposits — without caring about the specific token address — should use
    /// this instead of fetching and discarding the `Address` from
    /// `get_settlement_token`.
    ///
    /// Read-only and auth-free: it performs no state mutation (no TTL write is
    /// needed for the simple binding key).
    ///
    /// # Returns
    /// * `true` if a settlement token is bound
    /// * `false` if no settlement token has been bound yet
    pub fn is_settlement_token_bound(env: Env) -> bool {
        Self::read_settlement_token(&env).is_some()
    }

    // ── Initialization ───────────────────────────────────────────────────────

    /// Initializes the escrow contract with the operational admin.
    ///
    /// Single-use. Stores the admin address that controls pause, emergency,
    /// protocol-fee, and governance operations. All escrow lifecycle operations
    /// (create, deposit, release, refund, cancel) call `require_initialized`
    /// so that these safety rails are always bound before money can move.
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

    /// Returns the stored governance admin address.
    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Admin)
    }

    /// Returns the protocol-wide hard-coded bounds used by validation paths.
    ///
    /// Callers and off-chain indexers should query this endpoint to discover
    /// the limits enforced by `create_contract` without relying on hard-coded
    /// constants:
    ///
    /// - `max_milestones`: maximum number of milestones per contract.
    /// - `max_single_milestone_stroops`: maximum amount for any single milestone.
    /// - `max_total_escrow_stroops`: maximum sum of all milestone amounts.
    /// - `max_fee_bps`: protocol fee ceiling in basis points (10 000 = 100 %).
    ///
    /// These are compile-time constants — the return value never changes
    /// between calls on the same contract binary. The function is read-only
    /// and requires no authorization.
    ///
    /// # Returns
    /// A [`ContractBounds`] value containing only limit fields. Unlike
    /// [`get_contract_summary`], this type carries no per-contract participant
    /// or accounting data and its schema version tracks the limits API only.
    pub fn get_bounds(_env: Env) -> ContractBounds {
        ContractBounds {
            max_milestones: MAX_MILESTONES,
            max_single_milestone_stroops: crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS,
            max_total_escrow_stroops: MAX_TOTAL_ESCROW_STROOPS,
            max_fee_bps: 10_000,
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
    pub fn get_mainnet_readiness_info(env: Env) -> ReadinessChecklist {
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

        // Validate all contract-local preconditions before any SAC transfer so
        // rejected deposits cannot debit the client and then fail state checks.
        let validated = deposit::validate_deposit(&env, contract_id, &caller, amount);

        let token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured));

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&caller, &env.current_contract_address(), &amount);

        deposit::apply_validated_deposit(&env, contract_id, caller, validated)
    }

    /// Finalize an escrow contract by writing immutable close metadata.
    ///
    /// `finalizer` must authorize the call and must be the stored client,
    /// freelancer, or assigned arbiter. Finalization is allowed only while the
    /// contract is `Completed` or `Disputed`. Once finalized, future
    /// contract-specific mutations fail with `AlreadyFinalized`.
    ///
    /// # Errors
    /// - `ContractPaused` when pause or emergency controls are active.
    /// - `ContractNotFound` when `contract_id` is unknown.
    /// - `AlreadyFinalized` when a close record already exists.
    /// - `UnauthorizedRole` when `finalizer` is not a contract participant.
    /// - `InvalidStatusTransition` unless status is `Completed` or `Disputed`.
    pub fn finalize_contract(env: Env, contract_id: u32, finalizer: Address) -> bool {
        finalize::finalize_contract_impl(&env, contract_id, finalizer)
    }

    /// Return immutable close metadata for `contract_id`, if it has been finalized.
    pub fn get_finalization_record(
        env: Env,
        contract_id: u32,
    ) -> Option<finalize::FinalizationRecord> {
        finalize::get_finalization_record_impl(&env, contract_id)
    }

    /// Propose a client migration for an existing contract.
    ///
    /// Canonical public entrypoint; delegates to `propose_client_migration_impl`.
    /// The current client must authorize the call. The proposed client address
    /// must not be the freelancer or the current client. The pending migration
    /// is stored in temporary storage with TTL.
    pub fn propose_client_migration(
        env: Env,
        contract_id: u32,
        current_client: Address,
        new_client: Address,
    ) -> bool {
        Self::require_not_paused(&env);
        // Delegate to migration module implementation
        migration::propose_client_migration_impl(&env, contract_id, current_client, new_client)
    }

    /// Accept a live pending client migration and update the contract.
    ///
    /// Canonical public entrypoint; delegates to `accept_client_migration_impl`.
    /// Only the proposed client address may authorize acceptance.
    pub fn accept_client_migration(env: Env, contract_id: u32, new_client: Address) -> bool {
        Self::require_not_paused(&env);
        migration::accept_client_migration_impl(&env, contract_id, new_client)
    }

    /// Return true if a live pending client migration exists.
    ///
    /// Canonical public entrypoint; delegates to `has_pending_client_migration_impl`.
    pub fn has_pending_client_migration(env: Env, contract_id: u32) -> bool {
        migration::has_pending_client_migration_impl(&env, contract_id)
    }

    /// Return the live pending client migration record.
    ///
    /// Canonical public entrypoint; delegates to `get_pending_client_migration_impl`.
    /// Panics with `InvalidState` when no live pending migration exists.
    pub fn get_pending_client_migration(env: Env, contract_id: u32) -> PendingClientMigration {
        migration::get_pending_client_migration_impl(&env, contract_id)
    }

    /// Approves a milestone for release.
    ///
    /// Records the caller's approval in temporary storage with a TTL of
    /// `PENDING_APPROVAL_TTL_LEDGERS` (~7 days). Each call resets the TTL.
    /// Duplicate approvals from the same party are rejected.
    ///
    /// Required approvers per mode:
    /// - `ClientOnly` — client only
    /// - `ArbiterOnly` — arbiter only
    /// - `ClientAndArbiter` — client or arbiter (one is enough)
    /// - `MultiSig` — both client and freelancer must approve
    ///
    /// # Errors
    /// * `ContractPaused` - If the contract is paused while not in emergency mode
    /// * `EmergencyActive` - If the contract is in an active emergency pause
    /// * `AlreadyFinalized` - If the contract has already been finalized
    /// * Approval/auth/state errors bubbled up from `approvals::approve_milestone`
    ///
    /// # Security
    /// * Pause/emergency gate runs BEFORE finalization checks, auth, TTL extension,
    ///   and approval staging so no approval state mutates while the contract is frozen.
    ///
    /// See `docs/escrow/approvals-and-release.md` for the full flow.
    pub fn approve_milestone_release(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
    ) -> bool {
        Self::require_not_paused(&env);
        Self::require_not_finalized(&env, contract_id);
        approvals::approve_milestone(&env, contract_id, milestone_index, &caller)
            .unwrap_or_else(|e| env.panic_with_error(e))
    }

    /// Grants exactly one pending reputation credit to the freelancer.
    ///
    /// This is called exactly once when a contract successfully transitions to
    /// the `Completed` state, either through the final milestone release
    /// or via dispute resolution. Credits accumulate independently for each
    /// completed contract and are consumed one at a time by `issue_reputation`.
    /// A `Refunded` contract never calls this helper and therefore earns no credit.
    fn grant_pending_reputation_credit(env: &Env, freelancer: &Address) {
        let pending_key = DataKey::PendingReputationCredits(freelancer.clone());
        let pending: i128 = env.storage().persistent().get(&pending_key).unwrap_or(0);
        env.storage().persistent().set(&pending_key, &(pending + 1));
    }

    /// Releases a specific milestone, transferring the net payout to the freelancer.
    ///
    /// Executes `SAC::transfer(from: escrow_address, to: freelancer, milestone.amount − fee)`.
   
