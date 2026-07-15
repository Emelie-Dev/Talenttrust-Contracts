use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::{safe_add_amounts, Contract, ContractStatus, DataKey, DisputeResolution, DisputeSplit, Escrow, Error};

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
/// - `InvalidDisputeSplit` for Split variant with negative legs or non-conserving sum
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
            // freelancer gets floor(available * 30 / 100), client gets remainder
            let freelancer_payout = available
                .checked_mul(30)
                .and_then(|value| value.checked_div(100))
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
/// Returns `Refunded` only when the full deposit has been refunded.
/// Otherwise returns `Completed`.
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
    /// Raise a dispute on a funded or partially funded escrow.
    /// Only the client or freelancer may call this.
    pub fn raise_dispute(env: Env, contract_id: u32, caller: Address) -> bool {
        Self::require_not_paused(&env);
        caller.require_auth();

        let key = DataKey::Contract(contract_id);
        let mut contract = env
            .storage()
            .persistent()
            .get::<_, Contract>(&key)
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        if caller != contract.client && caller != contract.freelancer {
            env.panic_with_error(Error::UnauthorizedRole);
        }
        if contract.arbiter.is_none() {
            env.panic_with_error(Error::ArbiterRequired);
        }
        if contract.status != ContractStatus::Funded
            && contract.status != ContractStatus::PartiallyFunded
        {
            env.panic_with_error(Error::InvalidState);
        }

        contract.status = ContractStatus::Disputed;
        env.storage().persistent().set(&key, &contract);

        env.events().publish(
            (symbol_short!("dispute"), contract_id),
            (caller, env.ledger().timestamp()),
        );
        true
    }

    /// Resolve a disputed escrow. Only the assigned arbiter may call this.
    pub fn resolve_dispute(
        env: Env,
        contract_id: u32,
        arbiter: Address,
        resolution: DisputeResolution,
    ) -> bool {
        Self::require_not_paused(&env);
        arbiter.require_auth();

        let key = DataKey::Contract(contract_id);
        let mut contract = env
            .storage()
            .persistent()
            .get::<_, Contract>(&key)
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        if contract.status != ContractStatus::Disputed {
            env.panic_with_error(Error::InvalidState);
        }
        if contract.arbiter.clone() != Some(arbiter.clone()) {
            env.panic_with_error(Error::UnauthorizedRole);
        }

        let (client_payout, freelancer_payout) = resolution_payouts(&contract, &resolution)
            .unwrap_or_else(|err| env.panic_with_error(err));

        contract.refunded_amount = safe_add_amounts(contract.refunded_amount, client_payout)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
        contract.released_amount = safe_add_amounts(contract.released_amount, freelancer_payout)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));

        if safe_add_amounts(contract.released_amount, contract.refunded_amount)
            != Some(contract.funded_amount)
        {
            env.panic_with_error(Error::AccountingInvariantViolated);
        }

        contract.status = final_status_after_resolution(&contract);
        env.storage().persistent().set(&key, &contract);

        env.events().publish(
            (symbol_short!("dsp_res"), contract_id),
            (
                arbiter,
                resolution.code(),
                client_payout,
                freelancer_payout,
                env.ledger().timestamp(),
            ),
        );
        true
    }
}
