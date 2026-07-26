//! Dispute entrypoints, payout arithmetic, and final-status helpers.
//!
//! `resolution_payouts` and `final_status_after_resolution` are pure: they
//! compute how the currently available escrow balance should be split for a
//! `DisputeResolution` and tell the caller whether the contract should end as
//! `Completed` or `Refunded`, without touching storage. `raise_dispute` and
//! `resolve_dispute` are the root entrypoints that own authentication, the
//! `Disputed` status transition, and writes to `DataKey::Contract(contract_id)`.

use soroban_sdk::{symbol_short, Address, Env};

use crate::{
    safe_add_amounts, Contract, ContractStatus, DataKey, DisputeResolution, DisputeSplit, Error,
    Escrow, EscrowArgs, EscrowClient, PARTIAL_REFUND_DENOMINATOR, PARTIAL_REFUND_FREELANCER_SHARE,
};

// ---------------------------------------------------------------------------
// disputes configuration helpers
// ---------------------------------------------------------------------------

/// Read-only getter for disputes configuration without mutating storage.
/// Returns sensible default (`partial_refund_freelancer_share_bps = 3000`, `partial_refund_client_share_bps = 7000`)
/// before initialization or if storage is unconfigured.
pub fn get_dispute_config(env: &Env) -> Option<DisputeConfig> {
    env.storage()
        .persistent()
        .get(&DataKey::DisputeConfigKey)
}

/// Storage writer for disputes configuration.
pub fn set_dispute_config(env: &Env, config: DisputeConfig) -> bool {
    env.storage()
        .persistent()
        .set(&DataKey::DisputeConfigKey, &config);
    true
}

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
) -> Result<(i128, i128), EscrowError> {
    let available = amount_validation::checked_available_balance(
        contract.funded_amount,
        contract.released_amount,
        contract.refunded_amount,
    )?;

    match resolution {
        DisputeResolution::FullRefund => Ok((available, 0)),
        DisputeResolution::PartialRefund => {
            // freelancer gets floor(available * PARTIAL_REFUND_FREELANCER_SHARE / PARTIAL_REFUND_DENOMINATOR), client gets remainder
            let freelancer_payout = available
                .checked_mul(PARTIAL_REFUND_FREELANCER_SHARE)
                .and_then(|value| value.checked_div(PARTIAL_REFUND_DENOMINATOR))
                .ok_or(Error::PotentialOverflow)?;
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
