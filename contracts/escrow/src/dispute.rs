//! Dispute entrypoints, payout arithmetic, and final-status helpers.
//!
//! `resolution_payouts` and `final_status_after_resolution` are pure: they
//! compute how the currently available escrow balance should be split for a
//! `DisputeResolution` and tell the caller whether the contract should end as
//! `Completed` or `Refunded`, without touching storage. `raise_dispute` and
//! `resolve_dispute` are the root entrypoints that own authentication, the
//! `Disputed` status transition, and writes to `DataKey::Contract(contract_id)`.

use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::{
    safe_add_amounts, ttl, Contract, ContractStatus, DataKey, DisputeResolution, Error, Escrow,
    EscrowArgs, EscrowClient,
};

// ---------------------------------------------------------------------------
// resolution_payouts: pure arithmetic for dispute payout calculations
// ---------------------------------------------------------------------------

/// Compute the payout split for a dispute resolution.
///
/// Returns `(client_payout, freelancer_payout)` where both values are non-negative
/// and sum to the available balance. The available balance is computed as:
/// `available = funded_amount - released_amount - refunded_amount`.
///
/// # Errors
/// - `AccountingInvariantViolated` if available would be negative (corrupted state)
/// - `PotentialOverflow` if intermediate calculations overflow
/// - `InvalidDisputeSplit` for Split variant with negative legs, components
///   that individually exceed `available`, or whose non-overflowing sum does
///   not exactly match `available`
///
/// # Example
/// ```ignore
/// use soroban_sdk::{Address, Env};
/// use crate::{
///     Contract, ContractStatus, DisputeResolution, DisputeSplit, ReleaseAuthorization,
/// };
///
/// let env = Env::default();
/// let contract = Contract {
///     client: Address::generate(&env),
///     freelancer: Address::generate(&env),
///     arbiter: Some(Address::generate(&env)),
///     status: ContractStatus::Disputed,
///     total_deposited: 100,
///     funded_amount: 100,
///     released_amount: 0,
///     refunded_amount: 0,
///     release_authorization: ReleaseAuthorization::ClientOnly,
///     reputation_issued: false,
/// };
///
/// // FullRefund routes every available stroop to the client.
/// assert_eq!(
///     resolution_payouts(&contract, &DisputeResolution::FullRefund),
///     Ok((100, 0))
/// );
///
/// // PartialRefund applies the 70/30 split, with floor rounding on the
/// // freelancer leg (client receives the whole remainder).
/// assert_eq!(
///     resolution_payouts(&contract, &DisputeResolution::PartialRefund),
///     Ok((70, 30))
/// );
///
/// // FullPayout routes every available stroop to the freelancer.
/// assert_eq!(
///     resolution_payouts(&contract, &DisputeResolution::FullPayout),
///     Ok((0, 100))
/// );
///
/// // Split accepts custom amounts that exactly conserve the available balance.
/// let split = DisputeSplit {
///     client_amount: 65,
///     freelancer_amount: 35,
/// };
/// assert_eq!(
///     resolution_payouts(&contract, &DisputeResolution::Split(split)),
///     Ok((65, 35))
/// );
/// ```
pub fn resolution_payouts(
    contract: &Contract,
    resolution: &DisputeResolution,
) -> Result<(i128, i128), Error> {
    let available = contract
        .funded_amount
        .checked_sub(contract.released_amount)
        .and_then(|value| value.checked_sub(contract.refunded_amount))
        .ok_or(Error::AccountingInvariantViolated)?;
    if available < 0 {
        return Err(Error::AccountingInvariantViolated);
    }

    match resolution {
        DisputeResolution::FullRefund => Ok((available, 0)),
        DisputeResolution::PartialRefund => {
            // freelancer gets floor(available * NUMERATOR / DENOMINATOR), client gets remainder
            let freelancer_payout = available
                .checked_mul(PARTIAL_REFUND_FREELANCER_SHARE_NUMERATOR)
                .and_then(|value| value.checked_div(PARTIAL_REFUND_DENOMINATOR))
                .ok_or(Error::PotentialOverflow)?;
            Ok((available - freelancer_payout, freelancer_payout))
        }
        DisputeResolution::FullPayout => Ok((0, available)),
        DisputeResolution::Split(split) => {
            if split.client_amount < 0 || split.freelancer_amount < 0 {
                return Err(Error::InvalidDisputeSplit);
            }
            // Issue #572: Reject split resolution whose components are individually within but jointly exceed balance
            if split.client_amount > available || split.freelancer_amount > available {
                return Err(Error::InvalidDisputeSplit);
            }
            let total = safe_add_amounts(split.client_amount, split.freelancer_amount)
                .ok_or(Error::PotentialOverflow)?;
            if total > available || total != available {
                return Err(Error::InvalidDisputeSplit);
            }
            Ok((split.client_amount, split.freelancer_amount))
        }
    }
}

/// Determine the final contract status after dispute resolution.
///
/// Returns [`ContractStatus::Refunded`] only when every stroop ever deposited
/// has been refunded (`refunded_amount == funded_amount`). Otherwise returns
/// [`ContractStatus::Completed`] — including the case where some funds remain
/// escrowed after a dispute resolution.
///
/// # Example
/// ```ignore
/// use soroban_sdk::{Address, Env};
/// use crate::{Contract, ContractStatus, ReleaseAuthorization};
///
/// let env = Env::default();
/// let fixture = |funded: i128, refunded: i128| Contract {
///     client: Address::generate(&env),
///     freelancer: Address::generate(&env),
///     arbiter: Some(Address::generate(&env)),
///     status: ContractStatus::Disputed,
///     total_deposited: funded,
///     funded_amount: funded,
///     released_amount: 0,
///     refunded_amount: refunded,
///     release_authorization: ReleaseAuthorization::ClientOnly,
///     reputation_issued: false,
/// };
///
/// // Full refund of the deposit lands the contract in the Refunded terminal state.
/// assert_eq!(
///     final_status_after_resolution(&fixture(100, 100)),
///     ContractStatus::Refunded,
/// );
///
/// // Partial refund plus the released remainder keeps the contract Completed.
/// assert_eq!(
///     final_status_after_resolution(&fixture(100, 60)),
///     ContractStatus::Completed,
/// );
/// ```
pub fn final_status_after_resolution(contract: &Contract) -> ContractStatus {
    if contract.refunded_amount == contract.funded_amount {
        ContractStatus::Refunded
    } else {
        ContractStatus::Completed
    }
}

// ---------------------------------------------------------------------------
// raise_dispute / resolve_dispute entrypoints
// ---------------------------------------------------------------------------

#[contractimpl]
impl Escrow {
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
        // Gate: contract must have been initialized so pause and emergency rails
        // are always in scope before any state mutation can occur.
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        caller.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        // Verify caller is client or freelancer
        if caller != contract.client && caller != contract.freelancer {
            env.panic_with_error(Error::UnauthorizedRole);
        }

        // Require arbiter assignment
        if contract.arbiter.is_none() {
            env.panic_with_error(Error::ArbiterRequired);
        }

        // Verify contract is in a disputable state (Funded or PartiallyFunded)
        match contract.status {
            ContractStatus::Funded | ContractStatus::PartiallyFunded => {}
            _ => env.panic_with_error(Error::InvalidState),
        }

        contract.status = ContractStatus::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        ttl::extend_contract_ttl(&env, contract_id);

        env.events().publish(
            (symbol_short!("dispute"), symbol_short!("opened")),
            (contract_id, caller),
        );

        true
    }

    /// Resolves an open dispute by applying the arbiter-selected resolution.
    ///
    /// This entrypoint applies the dispute resolution (FullRefund, PartialRefund,
    /// FullPayout, or custom Split) to the remaining escrowed balance. The resolution
    /// must be authorized by the assigned arbiter and must conserve the available funds.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `contract_id` - The contract ID
    /// * `arbiter` - The arbiter address (must match contract's assigned arbiter)
    /// * `resolution` - The resolution decision (FullRefund, PartialRefund, FullPayout, or Split)
    ///
    /// # Returns
    /// `true` if the dispute was successfully resolved
    ///
    /// # Errors
    /// * `NotInitialized` - If `initialize` has not been called
    /// * `ContractNotFound` - If contract doesn't exist
    /// * `UnauthorizedRole` - If caller is not the assigned arbiter
    /// * `InvalidStatusTransition` - If contract is not in Disputed state
    /// * `InvalidDisputeSplit` - If custom split doesn't match available balance
    /// * `AccountingInvariantViolated` - If accounting state is inconsistent
    /// * `PotentialOverflow` - If amount calculations would overflow
    /// * `ContractPaused` - If pause or emergency controls are active
    /// * `AlreadyFinalized` - If contract has been finalized
    ///
    /// # Security
    /// - Only the assigned arbiter can resolve disputes
    /// - Split amounts must exactly match available balance
    /// - Updates released_amount and refunded_amount atomically
    /// - Emits dispute resolution event for indexers
    /// - Sets final contract status based on resolution outcome
    pub fn resolve_dispute(
        env: Env,
        contract_id: u32,
        arbiter: Address,
        resolution: DisputeResolution,
    ) -> bool {
        // Gate: contract must have been initialized so pause and emergency rails
        // are always in scope before any state mutation can occur.
        Self::require_initialized(&env);
        Self::require_not_paused(&env);
        arbiter.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        ttl::extend_contract_ttl(&env, contract_id);
        Self::require_not_finalized(&env, contract_id);

        // Verify contract is in Disputed state
        if contract.status != ContractStatus::Disputed {
            env.panic_with_error(Error::InvalidStatusTransition);
        }

        // Verify caller is the assigned arbiter
        match &contract.arbiter {
            Some(contract_arbiter) if *contract_arbiter == arbiter => {}
            _ => env.panic_with_error(Error::UnauthorizedRole),
        }

        // Compute payouts based on resolution
        let (client_payout, freelancer_payout) =
            resolution_payouts(&contract, &resolution).unwrap_or_else(|e| env.panic_with_error(e));

        // Update contract accounting
        contract.refunded_amount += client_payout;
        contract.released_amount += freelancer_payout;

        // Set final status
        contract.status = final_status_after_resolution(&contract);
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

        true
    }
}
