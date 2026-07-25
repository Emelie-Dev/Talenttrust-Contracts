use soroban_sdk::{contractimpl, contracttype, Address, Env};

use crate::{
    safe_add_amounts, Contract, ContractStatus, DataKey, Escrow, EscrowArgs, EscrowClient,
    EscrowError,
};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

#[contractimpl]
impl Escrow {
    /// Raise a dispute on a funded escrow. Only the client or freelancer may call this.
    pub fn raise_dispute(env: Env, contract_id: u32, caller: Address) -> bool {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::ContractPaused);
        }
        caller.require_auth();

        let key = DataKey::Contract(contract_id);
        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        if caller != contract.client && caller != contract.freelancer {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }
        if contract.arbiter.is_none() {
            env.panic_with_error(EscrowError::ArbiterRequired);
        }
        if contract.status != ContractStatus::Funded
            && contract.status != ContractStatus::PartiallyFunded
        {
            env.panic_with_error(EscrowError::InvalidStatusTransition);
        }

        let old_status = contract.status;
        contract.status = ContractStatus::Disputed;

        env.storage().persistent().set(&key, &contract);

        env.events().publish(
            (symbol_short!("dispute"), contract_id),
            (caller, env.ledger().timestamp()),
        );
        true
    }

    /// Resolve a disputed escrow and distribute the remaining balance according to the resolution.
    pub fn resolve_dispute(
        env: Env,
        contract_id: u32,
        arbiter: Address,
        resolution: DisputeResolution,
    ) -> bool {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::ContractPaused);
        }
        arbiter.require_auth();

        let key = DataKey::Contract(contract_id);
        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        if contract.status != ContractStatus::Disputed {
            env.panic_with_error(EscrowError::InvalidStatusTransition);
        }
        if contract.arbiter.clone() != Some(arbiter.clone()) {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }

        let old_status = contract.status;
        let (client_payout, freelancer_payout) = resolution_payouts(&contract, &resolution)
            .unwrap_or_else(|err| env.panic_with_error(err));

        contract.refunded_amount = safe_add_amounts(contract.refunded_amount, client_payout)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
        contract.released_amount = safe_add_amounts(contract.released_amount, freelancer_payout)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

        if safe_add_amounts(contract.released_amount, contract.refunded_amount)
            != Some(contract.funded_amount)
        {
            env.panic_with_error(EscrowError::AccountingInvariantViolated);
        }

        contract.status = final_status_after_resolution(&contract);
        if contract.status == ContractStatus::Completed {
            let pending_key = DataKey::PendingReputationCredits(contract.freelancer.clone());
            let pending: i128 = env.storage().persistent().get(&pending_key).unwrap_or(0);
            env.storage().persistent().set(&pending_key, &(pending + 1));
        }

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

/// Resolution selected by the assigned arbiter for a disputed escrow.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisputeResolution {
    /// Refund all remaining escrowed funds to the client.
    FullRefund,
    /// Refund 70% of the remaining balance to the client and release 30% to the freelancer.
    PartialRefund,
    /// Release all remaining escrowed funds to the freelancer.
    FullPayout,
    /// Apply a custom split of the remaining balance.
    Split(i128, i128),
}

#[contractimpl]
impl Escrow {
    pub fn raise_dispute(env: Env, contract_id: u32, caller: Address) -> bool {
        Self::require_not_paused(&env);
        caller.require_auth();

        let mut contract: Contract = env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        if caller != contract.client && caller != contract.freelancer {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }

        contract.status = ContractStatus::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Contract(contract_id), &contract);

        true
    }
}

impl DisputeResolution {
    pub fn code(&self) -> u32 {
        match self {
            Self::FullRefund => 0,
            Self::PartialRefund => 1,
            Self::FullPayout => 2,
            Self::Split(_, _) => 3,
        }
    }
}

#[allow(dead_code)]
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
            let freelancer_payout = available
                .checked_mul(30)
                .and_then(|value| value.checked_div(100))
                .ok_or(Error::PotentialOverflow)?;
            Ok((available - freelancer_payout, freelancer_payout))
        }
        DisputeResolution::FullPayout => Ok((0, available)),
        DisputeResolution::Split(client_amount, freelancer_amount) => {
            if *client_amount < 0 || *freelancer_amount < 0 {
                return Err(Error::InvalidDisputeSplit);
            }
            let total = safe_add_amounts(*client_amount, *freelancer_amount)
                .ok_or(Error::PotentialOverflow)?;
            if total != available {
                return Err(Error::InvalidDisputeSplit);
            }
            Ok((*client_amount, *freelancer_amount))
        }
    }
}

#[allow(dead_code)]
pub fn final_status_after_resolution(contract: &Contract) -> ContractStatus {
    if contract.refunded_amount == contract.funded_amount {
        ContractStatus::Refunded
    } else {
        ContractStatus::Completed
    }
}

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
