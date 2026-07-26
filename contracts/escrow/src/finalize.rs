use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

use crate::{
    safe_subtract_amounts, storage, Contract, ContractStatus, ContractSummary, DataKey, Error,
    Escrow, EscrowError, Milestone, MilestoneSummary, CONTRACT_SUMMARY_SCHEMA_VERSION,
};

/// Immutable metadata written when an escrow contract is closed.
///
/// The record is stored once under `DataKey::Finalization(contract_id)`.
/// After it exists, all contract-specific mutating entrypoints reject with
/// `Error::AlreadyFinalized`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizationRecord {
    /// Authorized client, freelancer, or assigned arbiter that finalized.
    pub finalizer: Address,
    /// Ledger timestamp at finalization time.
    pub timestamp: u64,
    /// Snapshot of participant, milestone, and accounting state.
    pub summary: ContractSummary,
}

impl Escrow {
    fn finalization_key(contract_id: u32) -> DataKey {
        settlement::finalization_key(contract_id)
    }

    fn load_contract_for_finalization(env: &Env, contract_id: u32) -> Contract {
        storage::load_contract(env, contract_id)
    }

    pub(crate) fn is_finalized(env: &Env, contract_id: u32) -> bool {
        storage::is_finalized(env, contract_id)
    }

    pub(crate) fn require_not_finalized(env: &Env, contract_id: u32) {
        storage::require_not_finalized(env, contract_id);
    }

    /// Load a contract from persistent storage, extend its TTL, and assert it
    /// has not been finalized.
    ///
    /// This is the canonical shared precondition for every state-changing
    /// entrypoint that operates on an existing escrow contract:
    ///
    /// 1. **Load** — reads `DataKey::Contract(contract_id)` from persistent
    ///    storage.  Panics with [`Error::ContractNotFound`] when the key is
    ///    absent.
    /// 2. **Extend TTL** — calls [`ttl::extend_contract_ttl`] so that active
    ///    contracts are not evicted from ledger state while they are being
    ///    mutated.
    /// 3. **Finalization guard** — calls [`Self::require_not_finalized`], which
    ///    panics with [`Error::AlreadyFinalized`] when a
    ///    [`DataKey::Finalization`] record exists for the contract.
    ///
    /// Callers that do **not** need the finalization check (currently only
    /// `issue_reputation`) should call [`ttl::extend_contract_ttl`] and the
    /// storage `get` directly instead of using this helper.
    ///
    /// # Errors
    /// * [`Error::ContractNotFound`]  — `contract_id` is not in storage.
    /// * [`Error::AlreadyFinalized`]  — the contract has been finalized.
    pub(crate) fn load_and_check_contract(env: &Env, contract_id: u32) -> Contract {
        let contract: Contract = env
            .storage()
            .persistent()
            .get(&crate::DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));
        crate::ttl::extend_contract_ttl(env, contract_id);
        Self::require_not_finalized(env, contract_id);
        contract
    }

    pub(crate) fn require_not_paused(env: &Env) {
        storage::require_not_paused(env);
    }

    /// Load a contract for mutation, applying storage precondition checks.
    ///
    /// This helper combines three essential preconditions into a single call:
    /// 1. Verifies contract operations are not paused or in emergency mode
    /// 2. Loads the contract from persistent storage
    /// 3. Verifies the contract has not been finalized (immutable)
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID to load
    ///
    /// # Panics
    /// - `ContractPaused` if the contract is paused
    /// - `EmergencyActive` if emergency mode is active
    /// - `ContractNotFound` if no contract exists for this ID
    /// - `AlreadyFinalized` if the contract has been finalized
    ///
    /// # Returns
    /// The loaded `Contract` if all preconditions pass
    pub(crate) fn require_contract_mutable(env: &Env, contract_id: u32) -> Contract {
        Self::require_not_paused(env);
        let contract = Self::load_contract_for_finalization(env, contract_id);
        Self::require_not_finalized(env, contract_id);
        contract
    }

    fn require_finalizer_role(env: &Env, contract: &Contract, finalizer: &Address) {
        // A finalizer must be one of the three contract participants
        authorization::require_participant(env, finalizer, contract);
    }

    fn summarize_contract(env: &Env, contract_id: u32, contract: &Contract) -> ContractSummary {
        let milestones: Vec<Milestone> = storage::load_milestones(env, contract_id);

        let mut total_amount: i128 = 0;
        let mut released_milestone_count: u32 = 0;
        let mut milestone_summaries = Vec::new(env);

        for (index, ms) in milestones.iter().enumerate() {
            let idx = index as u32;
            total_amount = total_amount
                .checked_add(ms.amount)
                .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

            if ms.released {
                released_milestone_count = released_milestone_count
                    .checked_add(1)
                    .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
            }

            milestone_summaries.push_back(MilestoneSummary {
                index: idx,
                amount: ms.amount,
                released: ms.released,
                refunded: ms.refunded,
            });
        }

        let reputation_issued = env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::ReputationIssued(contract_id))
            .unwrap_or(false);

        ContractSummary {
            schema_version: 1,
            client: contract.client.clone(),
            freelancer: contract.freelancer.clone(),
            arbiter: contract.arbiter.clone(),
            status: contract.status,
            reputation_issued,
            total_amount,
            funded_amount: contract.funded_amount,
            released_amount: contract.released_amount,
            refundable_balance: contract.funded_amount
                - contract.released_amount
                - contract.refunded_amount,
            released_milestone_count,
            milestones: milestone_summaries,
        }
    }
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
pub fn finalize_contract_impl(env: &Env, contract_id: u32, finalizer: Address) -> bool {
    Escrow::require_not_paused(&env);
    finalizer.require_auth();

    let contract = Escrow::load_contract_for_finalization(&env, contract_id);
    Escrow::require_not_finalized(&env, contract_id);
    Escrow::require_finalizer_role(&env, &contract, &finalizer);

    if contract.status != ContractStatus::Completed && contract.status != ContractStatus::Disputed {
        env.panic_with_error(Error::InvalidStatusTransition);
    }

    let record = FinalizationRecord {
        finalizer: finalizer.clone(),
        timestamp: env.ledger().timestamp(),
        summary: Escrow::summarize_contract(&env, contract_id, &contract),
    };

    settlement::write_finalization(&env, contract_id, &record);

    if contract.status == ContractStatus::Disputed {
        crate::rollback::clear_dispute_rollback(env, contract_id);
    }

    env.events().publish(
        (symbol_short!("finalized"), contract_id),
        (finalizer, record.timestamp),
    );

    true
}

/// Return immutable close metadata for `contract_id`, if it has been finalized.
pub fn get_finalization_record_impl(env: &Env, contract_id: u32) -> Option<FinalizationRecord> {
    settlement::read_finalization(env, contract_id)
}
