use crate::{
    approvals, ttl, Contract, ContractStatus, DataKey, Error, Escrow, EscrowArgs, EscrowClient,
    Milestone, ReleaseAuthorization,
};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

#[contractimpl]
impl Escrow {
    /// Releases a specific milestone, transferring funds to the freelancer.
    ///
    /// Requires valid, non-expired approvals based on the contract's ReleaseAuthorization mode.
    ///
    /// MultiSig semantics are client-and-freelancer approval. A MultiSig
    /// milestone can be released only by the stored client or freelancer after
    /// both of those addresses have approved the same milestone.
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
    /// * `InsufficientFunds` - If contract doesn't have enough funded balance
    /// * `InsufficientApprovals` - If required approvals are missing
    /// * `ApprovalExpired` - If approvals have expired
    /// * `UnauthorizedRole` - If caller is not authorized to release
    ///
    /// # Security
    /// - Requires valid approvals that haven't expired
    /// - Approvals are cleared after successful release
    /// - Fail-closed: missing or expired approvals prevent release
    pub fn release_milestone(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_index: u32,
    ) -> bool {
        caller.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

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

        approvals::check_approvals(&env, &contract, contract_id, milestone_index)
            .unwrap_or_else(|e| env.panic_with_error(e));

        let milestone_key = Symbol::new(&env, "milestones");
        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), milestone_key.clone()))
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

        let available_balance =
            contract.funded_amount - contract.released_amount - contract.refunded_amount;
        if available_balance < milestone.amount {
            env.panic_with_error(Error::InsufficientFunds);
        }

        let _release_amount = milestone.amount;
        milestone.released = true;
        milestones.set(milestone_index, milestone.clone());
        contract.released_amount += milestone.amount;

        if is_initialized(&env) {
            let fee_bps = get_protocol_fee_bps(&env);
            if fee_bps > 0 {
                let fee = calculate_protocol_fee(milestone.amount, fee_bps);
                let current_accumulated: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::AccumulatedProtocolFees)
                    .unwrap_or(0);
                env.storage().persistent().set(
                    &DataKey::AccumulatedProtocolFees,
                    &(current_accumulated + fee),
                );
            }
        }

        approvals::clear_approvals(&env, contract_id, milestone_index);

        let all_released = milestones.iter().all(|m| m.released || m.refunded);
        if all_released {
            contract.status = ContractStatus::Completed;
        }

        env.storage().persistent().set(
            &(DataKey::Contract(contract_id), milestone_key),
            &milestones,
        );
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_and_milestones_ttl(&env, contract_id);

        true
    }

    /// Atomically releases a batch of milestones in a single transaction.
    ///
    /// Validates authorization and approvals for **every** index before mutating any
    /// state (all-or-nothing). If any index fails validation the entire call reverts.
    /// A single batch event is emitted with the total released amount.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `caller` - The address of the caller (must be authorized)
    /// * `milestone_indices` - Non-empty, deduplicated list of milestone indices to release
    ///
    /// # Returns
    /// Total amount released across all milestones in the batch
    ///
    /// # Errors
    /// * `EmptyBatchRelease` - If `milestone_indices` is empty
    /// * `DuplicateMilestoneInBatch` - If any index appears more than once
    /// * `ContractNotFound` - If the contract does not exist
    /// * `InvalidState` - If the contract is not in `Funded` status
    /// * `UnauthorizedRole` - If `caller` is not permitted under the contract's `ReleaseAuthorization`
    /// * `IndexOutOfBounds` - If any index exceeds the milestone count
    /// * `MilestoneAlreadyReleased` - If any milestone is already released
    /// * `AlreadyRefunded` - If any milestone is already refunded
    /// * `InsufficientApprovals` - If required approvals are missing for any index
    /// * `InsufficientFunds` - If aggregate funded balance is insufficient
    ///
    /// # Security
    /// - `caller.require_auth()` is called once; Soroban propagates it to all indices.
    /// - All validation (auth, approvals, state) runs before any state mutation —
    ///   no partial-release side-effects on rejection.
    /// - Approvals for every released milestone are cleared atomically.
    pub fn release_milestones_batch(
        env: Env,
        contract_id: u32,
        caller: Address,
        milestone_indices: Vec<u32>,
    ) -> i128 {
        // Reject empty requests up-front.
        if milestone_indices.is_empty() {
            env.panic_with_error(Error::EmptyBatchRelease);
        }

        // Reject duplicate indices (O(n²) is fine for the small batches expected here).
        for i in 0..milestone_indices.len() {
            for j in (i + 1)..milestone_indices.len() {
                if milestone_indices.get(i).unwrap() == milestone_indices.get(j).unwrap() {
                    env.panic_with_error(Error::DuplicateMilestoneInBatch);
                }
            }
        }

        // Authenticate the caller once.
        caller.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);

        Self::require_not_finalized(&env, contract_id);

        if contract.status != ContractStatus::Funded {
            env.panic_with_error(Error::InvalidState);
        }

        // Role check — same logic as release_milestone.
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

        let milestone_key = Symbol::new(&env, "milestones");
        let mut milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), milestone_key.clone()))
            .unwrap();

        ttl::extend_milestone_ttl(&env, contract_id);

        // ── Validation pass (all-or-nothing) ────────────────────────────────
        let mut total_amount: i128 = 0;
        for idx in milestone_indices.iter() {
            if idx >= milestones.len() {
                env.panic_with_error(Error::IndexOutOfBounds);
            }
            let m = milestones.get(idx).unwrap();
            if m.released {
                env.panic_with_error(Error::MilestoneAlreadyReleased);
            }
            if m.refunded {
                env.panic_with_error(Error::AlreadyRefunded);
            }
            approvals::check_approvals(&env, &contract, contract_id, idx)
                .unwrap_or_else(|e| env.panic_with_error(e));
            total_amount += m.amount;
        }

        let available_balance =
            contract.funded_amount - contract.released_amount - contract.refunded_amount;
        if available_balance < total_amount {
            env.panic_with_error(Error::InsufficientFunds);
        }

        // ── Mutation pass ────────────────────────────────────────────────────
        let mut total_fee: i128 = 0;
        let fee_bps = if is_initialized(&env) {
            get_protocol_fee_bps(&env)
        } else {
            0
        };

        for idx in milestone_indices.iter() {
            let mut m = milestones.get(idx).unwrap();
            m.released = true;
            milestones.set(idx, m.clone());
            contract.released_amount += m.amount;
            if fee_bps > 0 {
                total_fee += calculate_protocol_fee(m.amount, fee_bps);
            }
            approvals::clear_approvals(&env, contract_id, idx);
        }

        if total_fee > 0 {
            let accumulated: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::AccumulatedProtocolFees)
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&DataKey::AccumulatedProtocolFees, &(accumulated + total_fee));
            env.events().publish(
                (symbol_short!("protocol_fee"), symbol_short!("batch")),
                (contract_id, total_fee),
            );
        }

        let all_done = milestones.iter().all(|m| m.released || m.refunded);
        if all_done {
            contract.status = ContractStatus::Completed;
        }

        env.storage().persistent().set(
            &(DataKey::Contract(contract_id), milestone_key),
            &milestones,
        );
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_and_milestones_ttl(&env, contract_id);

        // Single batch release event with aggregated total.
        env.events().publish(
            (symbol_short!("release"), symbol_short!("batch")),
            (contract_id, total_amount),
        );

        total_amount
    }
}

/// Returns true if the contract has been initialized.
fn is_initialized(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get::<_, bool>(&DataKey::Initialized)
        .unwrap_or(false)
}

/// Returns the protocol fee in basis points.
fn get_protocol_fee_bps(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get::<_, u32>(&DataKey::ProtocolFeeBps)
        .unwrap_or(0)
}

/// Calculates the protocol fee for a given amount.
fn calculate_protocol_fee(amount: i128, fee_bps: u32) -> i128 {
    if fee_bps == 0 {
        return 0;
    }
    amount * fee_bps as i128 / 10_000
}
