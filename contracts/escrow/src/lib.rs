#![no_std]

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Symbol, Vec};

mod types;
pub use types::{
    ContractStatus, ContractSummary, DataKey, EscrowError, Milestone, MilestoneSummary,
    ReadinessChecklist, CONTRACT_SUMMARY_SCHEMA_VERSION,
};

mod amount_validation;
pub use amount_validation::{
    safe_add_amounts, safe_subtract_amounts, AmountValidationError,
};

mod ttl;
pub use ttl::{
    LEDGERS_PER_DAY, PENDING_APPROVAL_BUMP_THRESHOLD, PENDING_APPROVAL_TTL_LEDGERS,
    PENDING_MIGRATION_BUMP_THRESHOLD, PENDING_MIGRATION_TTL_LEDGERS,
};

// ─── Bounds constants ─────────────────────────────────────────────────────────

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

#[contractimpl]
impl Escrow {
    // ─── Guard ───────────────────────────────────────────────────────────────

    /// Panics with `ContractPaused` if the contract is paused or in emergency.
    fn require_not_paused(env: &Env) {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::ContractPaused);
        }
    }

    /// Panics with `NotInitialized` if `initialize` has not been called.
    fn require_initialized(env: &Env) {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::NotInitialized);
        }
    }

    /// Panics with `UnauthorizedRole` if `caller` is not the stored admin.
    fn require_admin(env: &Env, caller: &Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        if *caller != admin {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }
    }

    // ─── Initialization ───────────────────────────────────────────────────────

    /// One-time initialization. Sets the admin address.
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
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Initialized, &true);

        // Update readiness checklist
        let mut checklist = Self::load_checklist(&env);
        checklist.initialized = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);

        env.events().publish(
            (symbol_short!("init"), Symbol::new(&env, "admin_set")),
            (admin, env.ledger().timestamp()),
        );
        true
    }

    /// Returns the current admin address, or `None` if not initialized.
    pub fn get_admin(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Admin)
    }

    // ─── Pause controls ───────────────────────────────────────────────────────

    /// Pause all mutating operations. Admin only.
    pub fn pause(env: Env) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Paused, &true);
        env.events().publish(
            (symbol_short!("paused"), env.ledger().timestamp()),
            (admin,),
        );
        true
    }

    /// Resume operations. Admin only. Fails if emergency is active.
    pub fn unpause(env: Env) -> bool {
        Self::require_initialized(&env);
        // Cannot unpause while emergency is active
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Emergency)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::EmergencyActive);
        }
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Paused, &false);
        env.events().publish(
            (symbol_short!("unpaused"), env.ledger().timestamp()),
            (admin,),
        );
        true
    }

    /// Returns `true` if the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
    }

    // ─── Emergency controls ───────────────────────────────────────────────────

    /// Activate emergency pause. Sets both `Paused` and `Emergency` flags. Admin only.
    pub fn activate_emergency_pause(env: Env) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Paused, &true);
        env.storage().persistent().set(&DataKey::Emergency, &true);

        // Update readiness checklist
        let mut checklist = Self::load_checklist(&env);
        checklist.emergency_controls_enabled = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);

        env.events().publish(
            (symbol_short!("emergency"), Symbol::new(&env, "activated")),
            (admin, env.ledger().timestamp()),
        );
        true
    }

    /// Resolve emergency and clear both flags. Admin only.
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

        // Update readiness checklist
        let mut checklist = Self::load_checklist(&env);
        checklist.emergency_controls_enabled = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);

        env.events().publish(
            (symbol_short!("emergency"), Symbol::new(&env, "resolved")),
            (admin, env.ledger().timestamp()),
        );
        true
    }

    /// Returns `true` if the contract is in emergency mode.
    pub fn is_emergency(env: Env) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Emergency)
            .unwrap_or(false)
    }

    // ─── Mainnet readiness ────────────────────────────────────────────────────

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

    /// Create a new escrow contract. Blocked when paused.
    pub fn create_contract(
        env: Env,
        client: Address,
        freelancer: Address,
        milestone_amounts: Vec<i128>,
    ) -> u32 {
        Self::require_not_paused(&env);
        client.require_auth();

        if client == freelancer {
            env.panic_with_error(EscrowError::InvalidParticipant);
        }
        if milestone_amounts.is_empty() {
            env.panic_with_error(EscrowError::EmptyMilestones);
        }
        let max_milestones = Self::effective_max_milestones(&env);
        if milestone_amounts.len() > max_milestones {
            env.panic_with_error(EscrowError::TooManyMilestones);
        }

        let mut total: i128 = 0;
        for i in 0..milestone_amounts.len() {
            let amt = milestone_amounts.get(i).unwrap();
            if amt <= 0 {
                env.panic_with_error(EscrowError::InvalidMilestoneAmount);
            }
            total = safe_add_amounts(total, amt)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        }
        let max_escrow = Self::effective_max_escrow_stroops(&env);
        if total > max_escrow {
            env.panic_with_error(EscrowError::InvalidMilestoneAmount);
        }

        let id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1);
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &(id + 1));

        let data = EscrowContractData {
            client: client.clone(),
            freelancer: freelancer.clone(),
            arbiter: None,
            milestones: milestone_amounts,
            status: ContractStatus::Created,
            total_deposited: 0,
            released_amount: 0,
            refunded_amount: 0,
            reputation_issued: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Contract(id), &data);

        env.events().publish(
            (symbol_short!("created"), id),
            (client, freelancer, env.ledger().timestamp()),
        );
        id
    }

    /// Deposit funds into an escrow contract. Blocked when paused.
    pub fn deposit_funds(env: Env, contract_id: u32, amount: i128) -> bool {
        Self::require_not_paused(&env);

        if amount <= 0 {
            env.panic_with_error(EscrowError::InvalidDepositAmount);
        }

        let key = DataKey::Contract(contract_id);
        let mut contract = env
            .storage()
            .persistent()
            .get::<_, EscrowContractData>(&key)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        contract.total_deposited =
            safe_add_amounts(contract.total_deposited, amount)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        if contract.status == ContractStatus::Created {
            contract.status = ContractStatus::Funded;
        }

        env.storage().persistent().set(&key, &contract);
        true
    }

    /// Release a milestone to the freelancer. Blocked when paused.
    pub fn release_milestone(env: Env, contract_id: u32, milestone_index: u32) -> bool {
        Self::require_not_paused(&env);

        let key = DataKey::Contract(contract_id);
        let mut contract = env
            .storage()
            .persistent()
            .get::<_, EscrowContractData>(&key)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        if milestone_index >= contract.milestones.len() {
            env.panic_with_error(EscrowError::InvalidMilestone);
        }

        let released_key = DataKey::MilestoneReleased(contract_id, milestone_index);
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&released_key)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::AlreadyReleased);
        }

        let milestone_amount = contract.milestones.get(milestone_index).unwrap();
        let available = contract.total_deposited
            - contract.released_amount
            - contract.refunded_amount;
        if available < milestone_amount {
            env.panic_with_error(EscrowError::InsufficientFunds);
        }

        env.storage().persistent().set(&released_key, &true);
        contract.released_amount =
            safe_add_amounts(contract.released_amount, milestone_amount)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        // Check if all milestones released → Completed
        let all_released = (0..contract.milestones.len()).all(|i| {
            env.storage()
                .persistent()
                .get::<_, bool>(&DataKey::MilestoneReleased(contract_id, i))
                .unwrap_or(false)
        });
        if all_released {
            contract.status = ContractStatus::Completed;
            // Increment pending reputation credits
            let credits_key = DataKey::PendingReputationCredits(contract.freelancer.clone());
            let credits: u32 = env
                .storage()
                .persistent()
                .get(&credits_key)
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&credits_key, &(credits + 1));
        }

        env.storage().persistent().set(&key, &contract);

        env.events().publish(
            (symbol_short!("released"), contract_id, milestone_index),
            (milestone_amount, env.ledger().timestamp()),
        );
        true
    }

    /// Issue reputation for a completed contract. Blocked when paused.
    pub fn issue_reputation(
        env: Env,
        contract_id: u32,
        caller: Address,
        freelancer: Address,
        rating: i128,
    ) -> bool {
        Self::require_not_paused(&env);
        caller.require_auth();

        let key = DataKey::Contract(contract_id);
        let mut contract = env
            .storage()
            .persistent()
            .get::<_, EscrowContractData>(&key)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        if caller != contract.client {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }
        if freelancer != contract.freelancer {
            env.panic_with_error(EscrowError::FreelancerMismatch);
        }
        if contract.status != ContractStatus::Completed {
            env.panic_with_error(EscrowError::NotCompleted);
        }
        if rating < 1 || rating > 5 {
            env.panic_with_error(EscrowError::InvalidRating);
        }

        let rep_key = DataKey::ReputationIssued(contract_id);
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&rep_key)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::ReputationAlreadyIssued);
        }
        env.storage().persistent().set(&rep_key, &true);

        contract.reputation_issued = true;
        env.storage().persistent().set(&key, &contract);

        let reputation_key = DataKey::Reputation(freelancer.clone());
        let mut record: ReputationRecord = env
            .storage()
            .persistent()
            .get(&reputation_key)
            .unwrap_or_default();
        record.total_rating += rating;
        record.completed_contracts += 1;
        record.last_rating = rating;
        env.storage().persistent().set(&reputation_key, &record);

        let credits_key = DataKey::PendingReputationCredits(freelancer.clone());
        let credits: u32 = env.storage().persistent().get(&credits_key).unwrap_or(0);
        if credits > 0 {
            env.storage()
                .persistent()
                .set(&credits_key, &(credits - 1));
        }

        env.events().publish(
            (symbol_short!("rep_issd"), contract_id),
            (freelancer, rating, env.ledger().timestamp()),
        );
        true
    }

    /// Cancel an escrow contract. Blocked when paused.
    pub fn cancel_contract(env: Env, contract_id: u32, caller: Address) -> bool {
        Self::require_not_paused(&env);
        caller.require_auth();

        let key = DataKey::Contract(contract_id);
        let mut contract = env
            .storage()
            .persistent()
            .get::<_, EscrowContractData>(&key)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        if contract.status == ContractStatus::Cancelled {
            env.panic_with_error(EscrowError::AlreadyCancelled);
        }
        if contract.status == ContractStatus::Completed {
            env.panic_with_error(EscrowError::InvalidStatusTransition);
        }

        let is_client = caller == contract.client;
        let is_freelancer = caller == contract.freelancer;
        if !is_client && !is_freelancer {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }

        contract.status = ContractStatus::Cancelled;
        env.storage().persistent().set(&key, &contract);

        env.events().publish(
            (symbol_short!("cancelled"), contract_id),
            (caller, env.ledger().timestamp()),
        );
        true
    }

    // ─── Read-only queries (not blocked by pause) ─────────────────────────────

    pub fn get_contract(env: Env, contract_id: u32) -> EscrowContractData {
        env.storage()
            .persistent()
            .get::<_, EscrowContractData>(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound))
    }

    pub fn get_reputation(env: Env, freelancer: Address) -> Option<ReputationRecord> {
        env.storage()
            .persistent()
            .get(&DataKey::Reputation(freelancer))
    }

    pub fn get_pending_reputation_credits(env: Env, freelancer: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingReputationCredits(freelancer))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod simple_amount_test;

#[cfg(test)]
mod test;
