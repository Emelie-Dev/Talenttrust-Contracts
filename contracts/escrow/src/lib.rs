//! TalentTrust escrow contract for milestone-based freelancer payments.
//!
//! The crate root exposes the Soroban contract struct and still owns the
//! entrypoints that don't warrant their own module: initialization,
//! settlement-token binding, reads, reputation, work evidence, and protocol
//! fee withdrawal. The escrow money-movement logic itself — releasing a
//! milestone, refunding it, cancelling a contract, and raising/resolving a
//! dispute — lives in dedicated modules (`release`, `refund`, `dispute`),
//! each contributing its own `#[contractimpl]` block to this same contract.
//! Supporting modules keep reusable validation, storage, governance, and
//! lifecycle helpers close to the paths that use them.
//!
//! ## Escrow source tree map
//!
//! | Source | Responsibility | Storage keys owned or touched |
//! | --- | --- | --- |
//! | `lib.rs` | Contract wrapper plus root entrypoints for setup, custody, reads, reputation, work evidence, pause/emergency, and fee withdrawal. | `DataKey::Initialized`, `Admin`, `SettlementToken`, `Paused`, `Emergency`, `ReadinessChecklist`, `Contract(id)`, `(Contract(id), "milestones")`, `MilestoneApprovals`, `AccumulatedProtocolFees`, `ReputationIssued`, `PendingReputationCredits`, `Reputation`, `ReputationComment` |
//! | `release` | `release_milestone` and `is_milestone_overdue` — the freelancer payout path, protocol-fee accrual, and milestone-deadline check. | `DataKey::Contract(id)`, `(Contract(id), "milestones")`, `AccumulatedProtocolFees`. |
//! | `refund` | `refund_unreleased_milestones` and `cancel_contract` — the client refund paths. | `DataKey::Contract(id)`, `(Contract(id), "milestones")`. |
//! | `dispute` | `raise_dispute` / `resolve_dispute` entrypoints plus the pure dispute payout arithmetic and final-status selection they use. | `DataKey::Contract(contract_id)`. |
//! | `amount_validation` | Stateless validation and checked arithmetic for stroop amounts and milestone totals. | None directly; callers write validated amounts to `Contract(id)` and milestone vectors. |
//! | `approvals` | Temporary milestone release approvals and release-authorization checks. | Temporary `DataKey::MilestoneApprovals(contract_id, milestone_index)`; reads `Contract(id)` and `(Contract(id), "milestones")`. |
//! | `deposit` | Deposit preflight and post-transfer accounting used by `deposit_funds`. | `DataKey::Contract(contract_id)` and `(DataKey::Contract(contract_id), "milestones")`. |
//! | `finalize` | Immutable finalization records, finalization guards, and final contract summaries. | `DataKey::Finalization(contract_id)`; reads `Contract(id)`, `(Contract(id), "milestones")`, `Paused`, and `Emergency`. |
//! | `migration` | Client migration proposals, acceptance checks, cancellation, and pending-migration reads. | Temporary `DataKey::PendingClientMigration(contract_id)`; reads and updates `DataKey::Contract(contract_id)`. |
//! | `rollback` | Guarded rollback of unchanged, unresolved disputes. | `DataKey::DisputeRollback(contract_id)`; reads and updates `DataKey::Contract(contract_id)` and its milestones. |
//! | `ttl` | TTL constants plus helpers for temporary and persistent storage renewal. | Extends caller-provided keys, especially `Contract(id)`, `(Contract(id), "milestones")`, `NextContractId`, participant indexes, approvals, and migrations. |
//! | `types` | Shared Soroban types, error enums, summaries, governance records, dispute records, and the canonical `DataKey` enum. | Declares storage key schema only; does not access storage itself. (New in this release: `DataKey::MaxDisputes`, `DataKey::DisputeCount(contract_id)`.) |
//! | `utils` | Small deterministic helpers shared by entrypoints, currently ledger timestamp access. | None. |
//! | `create_contract` | Contract creation, participant/milestone validation, ID allocation, and creation events. | `DataKey::Contract(id)`, `(DataKey::Contract(id), "milestones")`, `NextContractId`, and `GovernedParameters`. |
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
mod create_contract;
mod deposit;
mod dispute;
mod finalize;
mod governance;
mod migration;
mod storage;
mod ttl;
mod types;
mod utils;

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
pub use storage::{initialize_storage_version, ESCROW_STORAGE_VERSION};
pub use ttl::{ADMIN_ROTATION_MIN_DELAY_LEDGERS, PENDING_MIGRATION_TTL_LEDGERS};
// Canonical milestone-vector storage helpers (issue #701). Every module in
// the contract must route milestone reads/writes through these (defined in
// `ttl`) rather than constructing the composite `(DataKey::Contract(id),
// Symbol("milestones"))` key inline. Centralising access gives a single
// point of truth for the key shape, the missing-entry error path, and
// the persistent-TTL bump parameters used by every read and write.
pub use ttl::{
    load_milestones, milestone_storage_key, store_milestones, try_load_milestones,
};
// Keep shared storage keys and escrow domain types centralized in `types.rs`.
// `DisputeResolution` and `DisputeSplit` are defined once in `types.rs` and
// re-exported here; `dispute.rs` uses them via `crate::DisputeResolution`.
pub use milestones::{Milestone, MilestoneApprovals, MilestoneSummary, ReleaseAuthorization};
pub use types::{
    Contract, ContractBounds, ContractStatus, ContractSummary, DataKey, DepositMode,

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

/// Maximum number of items returned per page query.
pub const PAGE_CEILING: u32 = 100;

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



impl Escrow {
    pub(crate) fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
        if contract_id == 0 {
            env.panic_with_error(Error::InvalidContractId);
        }
    }

    /// Get the settlement token address from the canonical `DataKey` binding.
    pub(crate) fn read_settlement_token(env: &Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::SettlementToken)
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

    pub(crate) fn require_party(env: &Env, contract: &Contract, caller: &Address) {
        let is_client = caller == &contract.client;
        let is_freelancer = caller == &contract.freelancer;
        let is_arbiter = contract.arbiter.as_ref() == Some(caller);

        if is_client || is_freelancer || is_arbiter {
            return;
        }

        env.panic_with_error(Error::PartyNotAuthorized);
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
    /// On a successful, authorized bind this publishes a settlement bind event
    /// with an indexed short topic for efficient off-chain querying by indexers
    /// and monitoring dashboards.
    ///
    /// * Topics: `(symbol_short!("sttl_bind"),)`
    /// * Data: `(admin: Address, token: Address, timestamp: u64)`
    ///
    /// The event only fires after the write succeeds. Rejected binds
    /// (uninitialized, unauthorized, invalid token, self, admin) panic before
    /// this point and therefore publish nothing. All payload fields are public
    /// configuration.
    pub fn bind_settlement_token(env: Env, admin: Address, token: Address) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));

        if admin != stored_admin {
            env.panic_with_error(Error::UnauthorizedRole);
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

        // Emit after the binding write succeeds so indexers can track the bound
        // asset using an indexed short topic for efficient off-chain querying.
        env.events().publish(
            (symbol_short!("sttl_bind"),),
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

    /// Returns the current bounded settlement state.
    ///
    /// This auth-free reader uses stored values only and does not extend TTL or
    /// mutate storage. Before settlement is configured it returns the default,
    /// with no token and zero accrued fees.
    pub fn get_settlement_state(env: Env) -> SettlementState {
        SettlementState {
            token: Self::read_settlement_token(&env),
            accumulated_protocol_fees: env
                .storage()
                .persistent()
                .get(&DataKey::AccumulatedProtocolFees)
                .unwrap_or(0),
        }
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
            env.panic_with_error(EscrowError::AlreadyInitialized);
        }

        admin.require_auth();
        storage::initialize_storage_version(&env);
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

    /// Returns the protocol-wide bounds used by validation paths.
    ///
    /// Callers and off-chain indexers should query this endpoint to discover
    /// the limits enforced by `create_contract` without relying on hard-coded
    /// constants:
    ///
    /// - `max_milestones`: maximum number of milestones per contract.
    /// - `max_single_milestone_stroops`: maximum amount for any single milestone.
    /// - `max_total_escrow_stroops`: maximum sum of all milestone amounts.
    /// - `max_fee_bps`: protocol fee ceiling in basis points (10 000 = 100 %).
    /// - `max_disputes`: maximum number of disputes per contract.
    ///
    /// The `max_disputes` field is read from persistent storage and falls back
    /// to a default when no admin override has been stored.
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
            max_disputes: Self::effective_max_disputes(&_env),
        }
    }

    // ─── Configurable disputes limit ──────────────────────────────────

    /// Returns the effective max disputes, falling back to the default.
    fn effective_max_disputes(env: &Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MaxDisputes)
            .unwrap_or(DEFAULT_MAX_DISPUTES)
    }

    /// Returns the dispute count for a contract (0 if not tracked yet).
    fn get_dispute_count(env: &Env, contract_id: u32) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::DisputeCount(contract_id))
            .unwrap_or(0)
    }

    /// Increments the dispute count for a contract by 1.
    fn increment_dispute_count(env: &Env, contract_id: u32) {
        let count = Self::get_dispute_count(env, contract_id);
        env.storage()
            .persistent()
            .set(&DataKey::DisputeCount(contract_id), &(count + 1));
    }

    /// Set the max disputes limit. Admin only. Rejects out-of-range values.
    pub fn set_max_disputes(env: Env, max_disputes: u32) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();

        if max_disputes < MIN_MAX_DISPUTES || max_disputes > MAX_MAX_DISPUTES {
            env.panic_with_error(EscrowError::LimitOutOfRange);
        }

        env.storage()
            .persistent()
            .set(&DataKey::MaxDisputes, &max_disputes);

        env.events().publish(
            (symbol_short!("limits"), Symbol::new(&env, "max_disputes")),
            (max_disputes, env.ledger().timestamp()),
        );
        true
    }

    /// Returns the current max disputes limit (or the default if not set).
    pub fn get_max_disputes(env: Env) -> u32 {
        Self::effective_max_disputes(&env)
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
        Self::require_not_finalized(&env, contract_id);

        let validated = deposit::validate_deposit(&env, contract_id, &caller, amount);

        let token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::SettlementTokenNotConfigured));

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&caller, &env.current_contract_address(), &amount);

        deposit::apply_validated_deposit(&env, contract_id, caller, validated)
    }

    pub fn finalize_contract(env: Env, contract_id: u32, finalizer: Address) -> bool {
        finalize::finalize_contract_impl(&env, contract_id, finalizer)
    }

    /// Restore an unchanged, unresolved dispute to its pre-dispute status.
    pub fn rollback_dispute(env: Env, contract_id: u32) -> bool {
        rollback::rollback_dispute_impl(&env, contract_id)
    }

    /// Return immutable close metadata for `contract_id`, if it has been finalized.
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

    // ── Versioned state migration ─────────────────────────────────────────

    /// Returns the current versioned state, transparently upgrading from V1 on read.
    ///
    /// Reads the storage version marker from [`DataKey::StorageVersion`].
    /// When the marker is absent or indicates v1, the legacy [`StateV1`] layout
    /// is deserialized and promoted to [`StateV2`] (with `status` defaulting
    /// to `Created`).  When the marker indicates v2, the [`StateV2`] record
    /// is returned directly.
    ///
    /// This is a **read-only** operation — it does not persist the migrated
    /// state.  Call [`Self::migrate_state`] to commit the upgrade to storage.
    pub fn get_state(env: Env) -> StateV2 {
        let version: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StorageVersion)
            .unwrap_or(1);

        match version {
            2 => env
                .storage()
                .persistent()
                .get::<_, StateV2>(&DataKey::State)
                .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound)),
            _ => {
                let v1: StateV1 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::State)
                    .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
                StateV2 {
                    client: v1.client,
                    freelancer: v1.freelancer,
                    status: ContractStatus::Created,
                }
            }
        }
    }

    /// Migrates legacy v1 state to the current v2 layout and persists the result.
    ///
    /// Requires admin authorization. When the storage is already at the current
    /// version this is a no-op that returns `true`.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `admin` - The admin address (must match stored admin)
    ///
    /// # Returns
    /// `true` on success (including no-op when already v2).
    ///
    /// # Events
    /// Emits `("state_migrated", version)` with `(admin, timestamp)` payload
    /// when an actual migration occurs.
    pub fn migrate_state(env: Env, admin: Address) -> bool {
        admin.require_auth();

        let version: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StorageVersion)
            .unwrap_or(1);

        if version >= CURRENT_MILESTONE_VERSION {
            return true;
        }

        let v1: StateV1 = env
            .storage()
            .persistent()
            .get(&DataKey::State)
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        let v2 = StateV2 {
            client: v1.client,
            freelancer: v1.freelancer,
            status: ContractStatus::Created,
        };

        env.storage().persistent().set(&DataKey::State, &v2);
        env.storage()
            .persistent()
            .set(&DataKey::StorageVersion, &CURRENT_MILESTONE_VERSION);

        env.events().publish(
            (
                Symbol::new(&env, "state_migrated"),
                CURRENT_MILESTONE_VERSION,
            ),
            (admin, env.ledger().timestamp()),
        );

        true
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
        if milestone_index >= MAX_MILESTONES {
            env.panic_with_error(Error::IndexOutOfBounds);
        }
        Self::require_not_paused(&env);
        Self::require_not_finalized(&env, contract_id);
        approvals::approve_milestone(&env, contract_id, milestone_index, &caller)
            .unwrap_or_else(|e| env.panic_with_error(e))
    }

    /// Batch variant of [`approve_milestone_release`](Self::approve_milestone_release)
    /// that accepts a bounded vector of milestone indices.
    ///
    /// If the vector length exceeds [`MAX_BATCH_APPROVALS`], the call is rejected
    /// with [`EscrowError::BatchCapExceeded`]. Per-item semantics are preserved:
    /// each milestone index goes through the same authorization logic as the
    /// single-entrypoint, and events are emitted per item.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `caller` - The address of the caller (must be authorized)
    /// * `milestone_indices` - Bounded vector of milestone indices to approve
    ///
    /// # Errors
    /// * `BatchCapExceeded` - If `milestone_indices` length exceeds the cap
    /// * All errors from [`approve_milestone_release`](Self::approve_milestone_release)
    ///
    /// # Events
    /// Emits `("approve", contract_id)` with payload
    /// `(caller, milestone_index, timestamp)` for each successfully approved milestone.
    pub fn approve_milestone_release_batch(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_indices: Vec<u32>,
    ) -> bool {
        Self::require_not_paused(&env);
        Self::require_not_finalized(&env, contract_id);

        if milestone_indices.len() > MAX_BATCH_APPROVALS {
            env.panic_with_error(EscrowError::BatchCapExceeded);
        }

        for i in 0..milestone_indices.len() {
            let milestone_index = milestone_indices.get(i).unwrap();
            approvals::approve_milestone(&env, contract_id, milestone_index, &caller)
                .unwrap_or_else(|e| env.panic_with_error(e));

            env.events().publish(
                (symbol_short!("approve"), contract_id),
                (caller.clone(), milestone_index, env.ledger().timestamp()),
            );
        }

        true
    }

    /// Grants exactly one pending reputation credit to the freelancer.
    ///
    /// This is called exactly once when a contract successfully transitions to
    /// the `Completed` state, either through the final milestone release
    /// or via dispute resolution. Credits accumulate independently for each
    /// completed contract and are consumed one at a time by `issue_reputation`.
    /// A `Refunded` contract never calls this helper and therefore earns no credit.
    pub(crate) fn grant_pending_reputation_credit(env: &Env, freelancer: &Address) {
        let pending_key = DataKey::PendingReputationCredits(freelancer.clone());
        let pending: i128 = env.storage().persistent().get(&pending_key).unwrap_or(0);
        env.storage().persistent().set(&pending_key, &(pending + 1));
    }

    /// Releases a specific milestone, transferring the net payout to the freelancer.
    ///
    /// Executes `SAC::transfer(from: escrow_address, to: freelancer, milestone.amount − fee)`.
    /// The protocol fee is retained inside the contract under
    /// `DataKey::AccumulatedProtocolFees` and stays commingled with the escrow balance
    /// until `withdraw_protocol_fees` is called.
    ///
    /// See [`docs/escrow/sac-custody.md`](../../../docs/escrow/sac-custody.md) for the
    /// full custody model and accounting invariant.
    ///
    /// The target milestone must be fully funded through per-milestone deposit
    /// allocation before it can be released.
    ///
    /// Requires valid, non-expired approvals based on the contract's ReleaseAuthorization mode.
    ///
    /// MultiSig semantics are client-and-freelancer approval. A MultiSig
    /// milestone can be released only by the stored client or freelancer after
    /// both of those addresses have approved the same milestone.
    ///
    /// Approvals are cleared from temporary storage after a successful release.
    /// Missing or expired approvals are fail-closed — they produce
    /// `InsufficientApprovals` and the call panics without mutating state.
    ///
    /// See `approve_milestone_release`, `get_milestone_approvals`, and
    /// `docs/escrow/approvals-and-release.md` for the full flow.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `caller` - The address of the caller (must be authorized)
    /// * `milestone_index` - The index of the milestone to release
    ///
    /// # Returns
    /// `true` if release was successful
    ///
    /// # Errors
    /// * `ContractNotFound` - If contract doesn't exist
    /// * `InvalidState` - If contract is not in Funded state
    /// * `InvalidMilestone` - If milestone index is out of bounds
    /// * `AlreadyReleased` - If milestone was already released
    /// * `AlreadyRefunded` - If milestone was already refunded
    /// * `InsufficientFunds` - If the milestone or aggregate contract balance is underfunded
    /// * `InsufficientApprovals` - If required approvals are missing
    /// * `ApprovalExpired` - If approvals have expired
    /// * `UnauthorizedRole` - If caller is not authorized to release
    ///
    /// # Security
    /// - Requires valid approvals that haven't expired
    /// - Approvals are cleared after successful release
    /// - Fail-closed: missing or expired approvals prevent release
    ///
    /// # Events
    /// Emits `("mlstn_rls", contract_id)` with payload
    /// `(milestone_index, amount, fee, new_released_amount, caller, timestamp)`
    /// on every successful release.
    ///
    /// Additionally emits `("ctrct_cmp", contract_id)` with payload
    /// `(caller, timestamp)` when the release transitions the contract to
    /// `Completed` (i.e. all milestones are released or refunded).
    pub fn release_milestone(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
    ) -> bool {
        Self::require_not_paused(&env);
        // Authenticate caller before any state-dependent logic
        caller.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        // Extend TTL on contract read
        ttl::extend_contract_ttl(&env, contract_id);

        Self::require_not_finalized(&env, contract_id);

        // Verify contract is in Funded state before release (deposit transitions
        // Created → Funded when fully funded, so release must accept Funded).
        if contract.status != ContractStatus::Funded {
            env.panic_with_error(Error::InvalidState);
        }

        // Check caller is authorized for this release authorization mode
        let is_client = caller == contract.client;
        let is_freelancer = caller == contract.freelancer;
        let is_arbiter = contract.arbiter.as_ref() == Some(&caller);

        match contract.release_authorization {
            ReleaseAuthorization::ClientOnly => {
                if !is_client {
                    env.panic_with_error(Error::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::ArbiterOnly => {
                if !is_arbiter {
                    env.panic_with_error(Error::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::ClientAndArbiter => {
                if !is_client && !is_arbiter {
                    env.panic_with_error(Error::UnauthorizedRole);
                }
            }
            ReleaseAuthorization::MultiSig => {
                if !is_client && !is_freelancer {
                    env.panic_with_error(Error::UnauthorizedRole);
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
            env.panic_with_error(Error::AlreadyRefunded);
        }

        // Check for valid approvals
        approvals::check_approvals(&env, &contract, contract_id, milestone_index)
            .unwrap_or_else(|e| env.panic_with_error(e));

        let milestone_key = Symbol::new(&env, "milestones");
        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), milestone_key.clone()))
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
        let available =
            contract.funded_amount - contract.released_amount - contract.refunded_amount;
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
        let net_amount = gross_amount - protocol_fee;

        // The available balance must cover the full gross milestone amount
        // (net payout + fee) without dipping into already-accumulated fees or
        // other milestones' funds.
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

        // Transfer the net amount (gross minus fee) to the freelancer.
        // The fee portion remains in the contract's token balance and is
        // tracked separately in AccumulatedProtocolFees.
        let token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured));
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &env.current_contract_address(),
            &contract.freelancer,
            &net_amount,
        );

        // Accrue the fee into the protocol's accumulated balance.
        if protocol_fee > 0 {
            env.storage().persistent().set(
                &DataKey::AccumulatedProtocolFees,
                &(accumulated_fees + protocol_fee),
            );
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
        env.storage().persistent().set(&pending_key, &new_pending);
    }

    // `release_milestone` and `is_milestone_overdue` are implemented in
    // `contracts/escrow/src/release.rs` via their own `#[contractimpl]` block.

    // `refund_unreleased_milestones` is implemented in
    // `contracts/escrow/src/refund.rs` via its own `#[contractimpl]` block.

        if milestone_index >= milestones.len() {
            return false; // Index out of bounds, not overdue
        }

        let milestone = milestones.get(milestone_index).unwrap();

        // Return false if already released
        if milestone.released {
            return false;
        }

        // Return false if no deadline set
        match milestone.deadline {
            None => false,
            Some(deadline) => {
                // Overdue if now > deadline (strictly greater)
                now_seconds(&env) > deadline
            }
        }
    }

    /// Refunds unreleased milestones back to the client.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `milestone_indices` - Vector of milestone indices to refund
    ///
    /// # Returns
    /// The total amount refunded
    ///
    /// # Errors
    /// * `ContractNotFound` - If contract doesn't exist
    /// * `EmptyRefundRequest` - If milestone_indices is empty
    /// * `DuplicateMilestoneInRefund` - If the same milestone appears multiple times
    /// * `IndexOutOfBounds` - If any milestone index is out of bounds
    /// * `AlreadyReleased` - If any milestone was already released
    /// * `AlreadyRefunded` - If any milestone was already refunded
    /// * `InsufficientFunds` - If contract doesn't have enough balance to refund
    /// * `AlreadyFinalized` - If a finalization record already exists for this contract
    /// * `InvalidState` - If contract status is not Created, Funded, or Disputed
    pub fn refund_unreleased_milestones(
        env: Env,
        contract_id: u32,
        milestone_indices: Vec<u32>,
    ) -> i128 {
        Self::require_not_paused(&env);
        // Validate non-empty request
        if milestone_indices.is_empty() {
            env.panic_with_error(EscrowError::EmptyRefundRequest);
        }

        // Check for duplicates
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
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        // Extend TTL on contract read
        ttl::extend_contract_ttl(&env, contract_id);

        Self::require_not_finalized(&env, contract_id);

        // Only allow refunds while the contract is still in an active,
        // unreleased state. Cancelled, Completed, and Refunded contracts
        // must not be refundable again.
        if contract.status != ContractStatus::Created
            && contract.status != ContractStatus::Funded
            && contract.status != ContractStatus::Disputed
        {
            env.panic_with_error(EscrowError::InvalidState);
        }

        contract.client.require_auth();

        let mut milestones: Vec<Milestone> = ttl::load_milestones(&env, contract_id);

        let mut total_refund_amount: i128 = 0;

        // Validate all milestones first
        for idx in milestone_indices.iter() {
            if idx >= milestones.len() {
                env.panic_with_error(Error::IndexOutOfBounds);
            }

            let milestone = milestones.get(idx).unwrap();

            // SECURITY: Check if milestone is already released
            if milestone.released {
                env.panic_with_error(Error::AlreadyReleased);
            }

            // SECURITY: Check if milestone is already refunded
            if milestone.refunded {
                env.panic_with_error(Error::AlreadyRefunded);
            }

            // SECURITY: Check timeout refund conditions - milestone must be overdue if deadline is set
            if let Some(deadline) = milestone.deadline {
                // Milestone has a deadline - check if it's overdue
                if !Self::is_milestone_overdue(env.clone(), contract_id, idx) {
                    // Deadline set but milestone not yet overdue
                    env.panic_with_error(Error::MilestoneNotOverdue);
                }
                // SECURITY: is_milestone_overdue already verified: now > deadline AND unreleased
            }
            // If no deadline (None), allow refund anytime (backward compatibility)

            total_refund_amount += milestone.amount;
        }

        // Check if there's enough balance
        let available_balance =
            contract.funded_amount - contract.released_amount - contract.refunded_amount;
        if available_balance < total_refund_amount {
            env.panic_with_error(EscrowError::InsufficientFunds);
        }

        // Transfer tokens from contract to client
        let token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured));

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &env.current_contract_address(),
            &contract.client,
            &total_refund_amount,
        );

        // Mark milestones as refunded
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

        // Check if all unreleased milestones are refunded
        let all_refunded_or_released = milestones.iter().all(|m| m.released || m.refunded);
        if all_refunded_or_released {
            let all_refunded = milestones.iter().all(|m| m.refunded);
            if all_refunded {
                contract.status = ContractStatus::Refunded;
            } else {
                // Some released, some refunded
                contract.status = ContractStatus::Completed;
                Self::grant_pending_reputation_credit(&env, &contract.freelancer);
            }
        }

        ttl::store_milestones(&env, contract_id, &milestones);
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        // Extend TTL on contract write (milestone TTL already extended by store_milestones)
        ttl::extend_contract_ttl(&env, contract_id);

        // Emit `refunded` event after all state mutations succeed.
        //
        // Topics : `(symbol_short!("refunded"), contract_id: u32)`
        // Data   : `(total_refund_amount: i128, new_status: ContractStatus, timestamp: u64)`
        env.events().publish(
            (symbol_short!("refunded"), contract_id),
            (
                total_refund_amount,
                contract.status,
                env.ledger().timestamp(),
            ),
        );

        total_refund_amount
    }

    /// Checks whether a contract with the given ID exists in storage.
    ///
    /// This is a cheap, non-panicking existence probe that returns `true` if
    /// the contract record is present and `false` otherwise. Unlike `get_contract`,
    /// this function does **not** panic with `ContractNotFound` for missing IDs,
    /// making it safe for indexers and clients iterating over ID ranges.
    ///
    /// # Security
    /// This is a read-only operation that does **not** extend the contract's TTL.
    /// Probing for contract existence cannot be abused to keep entries alive.
    /// Only actual contract operations (reads/writes) extend TTL.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID to check
    ///
    /// # Returns
    /// * `true` if the contract exists
    /// * `false` if the contract does not exist
    ///
    /// # Examples
    /// ```ignore
    /// // Safe iteration over a range of IDs
    /// for id in 1..=100 {
    ///     if escrow.contract_exists(id) {
    ///         let contract = escrow.get_contract(id);
    ///         // process contract
    ///     }
    /// }
    /// ```
    pub fn contract_exists(env: Env, contract_id: u32) -> bool {
        storage::ensure_storage_version(&env);
        env.storage()
            .persistent()
            .has(&DataKey::Contract(contract_id))
    }

    /// Retrieves contract information.
    ///
    /// Transparently upgrades records still stored in a pre-`reputation_issued`
    /// legacy layout (schema version 1) to the current [`Contract`] layout on
    /// read; see `migration::migrate_contract_storage`.
    pub fn get_contract(env: Env, contract_id: u32) -> Contract {
        let contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        contract
    }

    /// Returns the next contract ID to be allocated (the high-water mark).
    ///
    /// This reader returns the current value of `NextContractId`, which represents
    /// the next ID that will be assigned when `create_contract` is called.
    /// Indexers can use this to determine the allocation high-water mark and
    /// safely iterate over the allocated ID range `[1, get_next_contract_id() - 1]`.
    ///
    /// # Security
    /// This is a read-only operation that does not mutate contract state or extend TTL.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    ///
    /// # Returns
    /// The next contract ID to be allocated (always ≥ 1)
    ///
    /// # Examples
    /// ```ignore
    /// // Get the high-water mark
    /// let next_id = escrow.get_next_contract_id();
    /// // All allocated IDs are in the range [1, next_id - 1]
    /// for id in 1..next_id {
    ///     if escrow.contract_exists(id) {
    ///         let contract = escrow.get_contract(id);
    ///         // process contract
    ///     }
    /// }
    /// ```
    pub fn get_next_contract_id(env: Env) -> u32 {
        storage::ensure_storage_version(&env);
        env.storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1)
    }

    /// Returns a bounded, paginated view of arbiter records.
    ///
    /// Enumerates contracts that have an assigned arbiter and returns
    /// [`ArbiterEntry`] values in ascending contract-id order. Contracts
    /// without an arbiter are skipped and do not consume a page slot.
    ///
    /// # Pagination
    ///
    /// - `start` is the zero-based offset into the filtered arbiter-record
    ///   sequence (not the raw contract-id space). An out-of-range `start`
    ///   produces an empty page (never a panic).
    /// - `limit` is clamped to `[0, PAGE_CEILING]` before use. The caller
    ///   never receives more than `PAGE_CEILING` entries per call.
    /// - Returns an empty `Vec` when no contracts exist or none have an
    ///   arbiter assigned.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `start` - Zero-based index of the first arbiter record in the page
    /// * `limit` - Maximum entries to return (clamped to `PAGE_CEILING`)
    ///
    /// # Returns
    /// A [`Vec<ArbiterEntry>`] containing at most `min(limit, PAGE_CEILING)`
    /// entries.
    ///
    /// # Side effects
    /// Extends the contract TTL for each returned entry, consistent with
    /// `get_contract`. Auth-free and otherwise non-mutating.
    pub fn get_arbiters_page(env: Env, start: u32, limit: u32) -> Vec<ArbiterEntry> {
        let capped_limit = core::cmp::min(limit, PAGE_CEILING);
        if capped_limit == 0 {
            return Vec::new(&env);
        }

        let next_id = Self::get_next_contract_id(env.clone());
        if next_id <= 1 {
            return Vec::new(&env);
        }

        let mut result = Vec::new(&env);
        let mut matched: u32 = 0;
        let mut collected: u32 = 0;
        let mut id: u32 = 1;

        while id < next_id && collected < capped_limit {
            if let Some(contract) = env
                .storage()
                .persistent()
                .get::<_, Contract>(&DataKey::Contract(id))
            {
                if let Some(arbiter) = contract.arbiter {
                    if matched >= start {
                        ttl::extend_contract_ttl(&env, id);
                        result.push_back(ArbiterEntry {
                            contract_id: id,
                            arbiter,
                        });
                        collected += 1;
                    }
                    matched = matched.saturating_add(1);
                }
            }
            id = id.saturating_add(1);
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
        storage::ensure_storage_version(&env);
        let contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

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
            .get(&(DataKey::Contract(contract_id), milestone_key))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
        ttl::extend_milestone_ttl(&env, contract_id);
        milestones
    }

    pub fn get_milestone(env: Env, contract_id: u32, milestone_index: u32) -> Option<Milestone> {
        Self::get_milestone_impl(&env, contract_id, milestone_index)
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
    /// Extends the milestones vector TTL on a successful read, consistent with
    /// `get_milestones`. Auth-free and otherwise non-mutating.
    pub fn get_milestone(env: Env, contract_id: u32, milestone_index: u32) -> Option<Milestone> {
        let milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), milestone_key))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
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
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
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
        Self::get_milestone_approvals_impl(&env, contract_id, milestone_index)
    }

    pub fn get_approval_deadline(env: Env, contract_id: u32, milestone_index: u32) -> Option<u32> {
        Self::get_approval_deadline_impl(&env, contract_id, milestone_index)
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
            env.panic_with_error(EscrowError::EmergencyActive);
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
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));

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
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
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

    pub fn get_mainnet_readiness_info(env: Env) -> MainnetReadinessInfo {
        let checklist = Self::load_checklist(&env);
        MainnetReadinessInfo {
            initialized: checklist.initialized,
            governed_params_set: checklist.governed_params_set,
            emergency_controls_enabled: checklist.emergency_controls_enabled,
            caps_set: MAINNET_MAX_TOTAL_ESCROW_PER_CONTRACT_STROOPS > 0,
            protocol_version: MAINNET_PROTOCOL_VERSION,
            max_escrow_total_stroops: MAINNET_MAX_TOTAL_ESCROW_PER_CONTRACT_STROOPS,
        }
    }

    fn load_checklist(env: &Env) -> ReadinessChecklist {
        env.storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default()
    }

    // ─── Configurable limits ──────────────────────────────────────────────────

    fn effective_max_milestones(env: &Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MaxMilestones)
            .unwrap_or(DEFAULT_MAX_MILESTONES)
    }

    fn effective_max_escrow_stroops(env: &Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::MaxEscrowStroops)
            .unwrap_or(DEFAULT_MAX_TOTAL_ESCROW_STROOPS)
    }

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

    pub fn get_max_milestones(env: Env) -> u32 {
        Self::effective_max_milestones(&env)
    }

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

    pub fn get_max_escrow_stroops(env: Env) -> i128 {
        Self::effective_max_escrow_stroops(&env)
    }

    // ── Admin: set arbiter ───────────────────────────────────────────────────

    pub fn set_arbiter(
        env: Env,
        contract_id: u32,
        admin: Address,
        new_arbiter: Option<Address>,
    ) -> bool {
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        Self::validate_contract_id_bounds(&env, contract_id);

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        if admin != stored_admin {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }
        admin.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if let Some(ref arb) = new_arbiter {
            if *arb == contract.client || *arb == contract.freelancer {
                env.panic_with_error(EscrowError::InvalidArbiter);
            }
        }

        if new_arbiter.is_none() {
            match contract.release_authorization {
                ReleaseAuthorization::ArbiterOnly | ReleaseAuthorization::ClientAndArbiter => {
                    env.panic_with_error(EscrowError::MissingArbiter);
                }
                _ => {}
            }
        }

        let old_arbiter = contract.arbiter.clone();
        contract.arbiter = new_arbiter.clone();

        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("arbiter"), contract_id),
            (old_arbiter, new_arbiter, env.ledger().timestamp()),
        );

        true
    }

    // ── Cancel contract ──────────────────────────────────────────────────────

    /// Cancels a contract before any milestone has been released.
    ///
    /// The caller must be the stored client and must authorize the call. The
    /// contract must be in `Created` or `Funded` state, with no released
    /// balance, and the full remaining refundable balance is sent back to the
    /// client via the configured Stellar Asset Contract before the contract is
    /// marked `Cancelled`. A zero-funded cancellation does not invoke a token
    /// transfer and leaves unrelated contracts' escrowed token balances intact.
    ///
    /// # Errors
    /// * `ContractPaused` - If the contract is paused while not in emergency mode.
    /// * `EmergencyActive` - If the contract is in an active emergency pause.
    /// * `ContractNotFound` - If the contract does not exist.
    /// * `UnauthorizedRole` - If the caller is not the stored client.
    /// * `AlreadyCancelled` - If the contract was already cancelled.
    /// * `InvalidStatusTransition` - If the contract is not `Created`/`Funded` or has already released funds.
    pub fn cancel_contract(env: Env, contract_id: u32, client: Address) -> bool {
        Self::require_not_paused(&env);
        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
        ttl::extend_contract_ttl(&env, contract_id);

        Self::require_not_finalized(&env, contract_id);

        if client != contract.client {
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

        client.require_auth();

        let refund_amount =
            contract.funded_amount - contract.released_amount - contract.refunded_amount;
        if refund_amount > 0 {
            let token = Self::read_settlement_token(&env)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &client,
                &refund_amount,
            );
        }
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);
        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("cancelled"), contract_id),
            (client, refund_amount, env.ledger().timestamp()),
        );

        true
    }

    // ── Dispute management ────────────────────────────────────────────────────

    // ── Reputation ───────────────────────────────────────────────────────────

    /// Issues reputation credit for a completed contract.
    ///
    /// # Comment length
    /// `comment` must be between 1 and 200 **bytes** (inclusive). Because Soroban
    /// `String::len()` returns the UTF-8 byte length, a multi-byte character (e.g.
    /// a 3-byte emoji) counts as 3 toward the limit. ASCII characters are 1 byte each.
    ///
    /// # Errors
    /// * `ContractPaused` - If the contract is paused while not in emergency mode
    /// * `EmergencyActive` - If the contract is in an active emergency pause
    /// * `ContractNotFound` - If contract doesn't exist
    /// * `UnauthorizedRole` - If caller is not the stored client
    /// * `FreelancerMismatch` - If `freelancer` does not match the stored freelancer
    /// * `InvalidRating` - If rating is not in [1, 5]
    /// * `EmptyComment` - If comment is 0 bytes
    /// * `CommentTooLong` - If comment exceeds 200 bytes
    /// * `NotCompleted` - If contract status is not `Completed`
    /// * `ReputationAlreadyIssued` - If reputation was already issued
    /// * `SelfRating` - If client and freelancer are the same address
    ///
    /// # Security
    /// * Pause/emergency gate runs BEFORE contract state read so paused
    ///   contracts cannot have reputation mutated while paused.
    /// * The 200-byte cap prevents unbounded on-chain storage growth.
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
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        ttl::extend_contract_ttl(&env, contract_id);

        if caller != contract.client {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }

        if rating < 1 || rating > 5 {
            env.panic_with_error(EscrowError::InvalidRating);
        }

        if comment.len() == 0 {
            env.panic_with_error(EscrowError::EmptyComment);
        }

        if comment.len() > 200 {
            env.panic_with_error(EscrowError::CommentTooLong);
        }

        if contract.status != ContractStatus::Completed {
            env.panic_with_error(EscrowError::NotCompleted);
        }

        if contract.reputation_issued {
            env.panic_with_error(EscrowError::ReputationAlreadyIssued);
        }
        if contract.client == contract.freelancer {
            env.panic_with_error(EscrowError::SelfRating);
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
            env.panic_with_error(EscrowError::InvalidState);
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

        env.events().publish(
            (symbol_short!("repr_put"), contract_id),
            (
                contract.freelancer.clone(),
                rating,
                env.ledger().timestamp(),
            ),
        );

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
        reputation_migration::read_reputation_with_migration(&env, &address)
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

    /// Migrate the reputation storage record for `address` to the current schema version.
    ///
    /// This entrypoint is idempotent: calling it on an already-current record is a
    /// safe no-op and returns `false`. When a v1 (legacy) record is detected the
    /// migration writes a [`DataKey::ReputationStorageVersion`] marker alongside the
    /// existing data and returns `true`. All field values are preserved exactly.
    ///
    /// # When to call
    ///
    /// Existing records written before versioning was introduced are transparently
    /// upgraded on every `get_reputation` read via the migration-on-read path, so
    /// most callers never need to call this directly. This explicit entrypoint is
    /// intended for operators who want to eagerly migrate a known address (e.g. as
    /// part of a deployment runbook) and receive a clear success/no-op signal.
    ///
    /// # Arguments
    ///
    /// * `address` — The freelancer address whose reputation record should be migrated.
    ///
    /// # Returns
    ///
    /// `true` if a migration was performed; `false` if the record was already at
    /// [`REPUTATION_STORAGE_VERSION`] or no record existed (no migration needed).
    ///
    /// # Security
    ///
    /// This is a permissionless read-equivalent: it does not transfer funds,
    /// change authorizations, or mutate business state beyond writing the version
    /// marker. Pause and emergency checks are intentionally omitted so operators
    /// can still migrate records during an incident pause.
    pub fn migrate_reputation_storage(env: Env, address: Address) -> bool {
        reputation_migration::migrate_reputation_storage_impl(&env, &address)
    }

    // -----------------------------------------------------------------------
    // Work evidence
    // -----------------------------------------------------------------------

    /// Records a deliverable reference (e.g. IPFS CID or URL hash) for an
    /// unreleased milestone.
    ///
    /// Only the contract's freelancer may call this. The contract must be in
    /// `Funded` status and the target milestone must not yet be released or
    /// refunded. Evidence may be overwritten before release.
    ///
    /// # Arguments
    /// * `contract_id` - The escrow contract to update
    /// * `caller`      - Must equal the stored `freelancer`; requires auth
    /// * `milestone_index` - Zero-based index of the milestone
    /// * `evidence`    - Deliverable reference; max 256 bytes
    ///
    /// # Errors
    /// * `NotInitialized`     — `initialize` has not been called
    /// * `ContractPaused` / `EmergencyActive` — pause/emergency gate
    /// * `ContractNotFound`   — unknown `contract_id`
    /// * `AlreadyFinalized`   — contract has been finalized
    /// * `UnauthorizedRole`   — `caller` is not the freelancer
    /// * `InvalidState`       — contract is not `Funded`
    /// * `IndexOutOfBounds`   — `milestone_index` exceeds milestone count
    /// * `MilestoneAlreadyReleased` — milestone is already released
    /// * `AlreadyRefunded`    — milestone has been refunded
    /// * `EvidenceTooLong`    — evidence string exceeds 256 bytes
    pub fn submit_work_evidence(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
        evidence: String,
    ) -> bool {
        /// Gate: contract must have been initialized so pause and emergency rails
        /// are always in scope before any state mutation can occur.
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        caller.require_auth();

        let contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        if caller != contract.freelancer {
            env.panic_with_error(Error::UnauthorizedRole);
        }

        if contract.status != ContractStatus::Funded {
            env.panic_with_error(EscrowError::InvalidState);
        }

        // Bound evidence to 256 bytes to prevent storage bloat.
        if evidence.len() > 256 {
            env.panic_with_error(Error::EvidenceTooLong);
        }

        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), milestone_key.clone()))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_milestone_ttl(&env, contract_id);

        if milestone_index >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let mut milestone = milestones.get(milestone_index).unwrap();

        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }
        if milestone.refunded {
            env.panic_with_error(Error::AlreadyRefunded);
        }

        milestone.work_evidence = Some(evidence.clone());
        milestones.set(milestone_index, milestone);

        ttl::store_milestones(&env, contract_id, &milestones);

        // Extend TTL on contract write (milestone TTL already extended by store_milestones)
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
        let milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&DataKey::Milestones(contract_id))
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

        let token = match Self::read_settlement_token(&env) {
            Some(t) => t,
            None => env.panic_with_error(EscrowError::SettlementTokenNotConfigured),
        };

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
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        product / 10_000
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
            env.panic_with_error(EscrowError::NotInitialized);
        }
    }

    pub(crate) fn is_initialized(env: &Env) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
    }

    // `raise_dispute` and `resolve_dispute` are implemented in
    // `contracts/escrow/src/dispute.rs` via their own `#[contractimpl]` block,
    // alongside the pure `resolution_payouts` / `final_status_after_resolution`
    // helpers that module already owned.
}

#[cfg(test)]
mod test;
