//! Dispute payout arithmetic and final-status helpers.
//!
//! This module is intentionally storage-free. It computes how the currently
//! available escrow balance should be split for a `DisputeResolution` and tells
//! the root dispute entrypoint whether the contract should end as `Completed`
//! or `Refunded`. The root entrypoints own authentication, token transfer, event
//! publication, and writes to `DataKey::Contract(contract_id)`.

use soroban_sdk::{symbol_short, Address, Env};

use crate::{
    amount_validation, safe_add_amounts, Contract, ContractStatus, DisputeResolution, DisputeSplit,
    EscrowError, MAX_SINGLE_AMOUNT_STROOPS,
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
/// - `InvalidDisputeSplit` for Split variant with negative legs or non-conserving sum
pub fn resolution_payouts(
    contract: &Contract,
    resolution: &DisputeResolution,
) -> Result<(i128, i128), EscrowError> {
    let available = amount_validation::checked_available_balance(
        contract.funded_amount,
        contract.released_amount,
        contract.refunded_amount,
    )?;

    match resolution {
        DisputeResolution::FullRefund => Ok((available, 0)),
        DisputeResolution::PartialRefund => {
            // freelancer gets floor(available * 30 / 100), client gets remainder
            let freelancer_payout = available
                .checked_mul(30)
                .and_then(|value| value.checked_div(100))
                .ok_or(EscrowError::PotentialOverflow)?;
            Ok((available - freelancer_payout, freelancer_payout))
        }
        DisputeResolution::FullPayout => Ok((0, available)),
        DisputeResolution::Split(split) => {
            if split.client_amount < 0 || split.freelancer_amount < 0 {
                return Err(EscrowError::InvalidDisputeSplit);
            }
            if split.client_amount > MAX_SINGLE_AMOUNT_STROOPS
                || split.freelancer_amount > MAX_SINGLE_AMOUNT_STROOPS
            {
                return Err(EscrowError::InvalidDisputeSplit);
            }
            if split.client_amount > available || split.freelancer_amount > available {
                return Err(EscrowError::InvalidDisputeSplit);
            }
            let total = safe_add_amounts(split.client_amount, split.freelancer_amount)
                .ok_or(EscrowError::PotentialOverflow)?;
            if total > available || total != available {
                return Err(EscrowError::InvalidDisputeSplit);
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
