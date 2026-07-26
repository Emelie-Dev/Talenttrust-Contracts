//! Dispute payout arithmetic and final-status helpers.
//!
//! This module is intentionally storage-free. It computes how the currently
//! available escrow balance should be split for a `DisputeResolution` and tells
//! the root dispute entrypoint whether the contract should end as `Completed`
//! or `Refunded`. The root entrypoints own authentication, token transfer, event
//! publication, and writes to `DataKey::Contract(contract_id)`.
//!
//! The two helpers exposed here, [`resolution_payouts`] and
//! [`final_status_after_resolution`], are pure: they take a `&Contract` plus a
//! [`crate::DisputeResolution`] and return a payout tuple or the post-resolution
//! status. Both are only invoked from [`crate::Escrow::resolve_dispute`] (the
//! arbiter-authorized resolution flow) — [`crate::Escrow::raise_dispute`] only
//! transitions the contract status to [`crate::ContractStatus::Disputed`] and
//! never touches the payout helpers. Everything in this module is
//! deterministic and free of host calls; authentication, token transfer, and
//! event publication remain in the storage-aware entrypoints in `lib.rs`.

/// Freelancer's share numerator for the PartialRefund dispute resolution (30%).
///
/// When a dispute is resolved with `PartialRefund`, the freelancer receives
/// `floor(available * PARTIAL_REFUND_FREELANCER_SHARE_NUMERATOR / PARTIAL_REFUND_DENOMINATOR)`
/// stroops and the client receives the remainder.  The current value of `30`
/// means the freelancer gets 30% of the available escrow balance.
pub const PARTIAL_REFUND_FREELANCER_SHARE_NUMERATOR: i128 = 30;

/// Denominator for the PartialRefund dispute resolution split.
///
/// Together with [`PARTIAL_REFUND_FREELANCER_SHARE_NUMERATOR`] this defines the
/// freelancer's share as a percentage.  With `NUMERATOR = 30` and
/// `DENOMINATOR = 100` the split is 30 % to the freelancer, 70 % to the client.
pub const PARTIAL_REFUND_DENOMINATOR: i128 = 100;

use soroban_sdk::{contractimpl, symbol_short, Address, Env};

use crate::{
    safe_add_amounts, Contract, ContractStatus, DataKey, DisputeResolution, DisputeSplit, Error,
    Escrow, EscrowArgs, EscrowClient,
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

// Dispute entrypoints are implemented in `contracts/escrow/src/lib.rs`.
// This module retains dispute-related helpers only.
