use crate::{
    DataKey, Escrow, EscrowArgs, EscrowClient, EscrowError, GovernedParameters, ReadinessChecklist,
};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

/// Governance-related privileged operations and audit events.
///
/// This module implements a small set of admin-facing functions that
/// produce parseable events for off-chain indexers. Events emitted here
/// follow the existing convention of short `symbol_short!` topics used by
/// other lifecycle events (e.g. `init`, `paused`, `emergency`).
#[contractimpl]
impl super::Escrow {
    /// Set the protocol fee (basis points). Emits an event with
    /// `(old_bps, new_bps, admin, timestamp)` under topic `protocol_fee_bps`.
    pub fn set_protocol_fee_bps(env: Env, new_bps: u32) -> bool {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&crate::DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::NotInitialized);
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(crate::Error::NotInitialized));
        admin.require_auth();

        let old_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ProtocolFeeBps)
            .unwrap_or(0u32);
        env.storage()
            .persistent()
            .set(&DataKey::ProtocolFeeBps, &new_bps);

        env.events().publish(
            (Symbol::new(env, "protocol_fee_bps"),),
            (old_bps, new_bps, admin.clone(), env.ledger().timestamp()),
        );
        true
    }

    /// Internal: propose a new admin with a timelock.
    pub(crate) fn propose_governance_admin_impl(env: Env, proposed: Address) -> bool {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&crate::DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::NotInitialized);
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(crate::Error::NotInitialized));
        admin.require_auth();

        env.storage().persistent().set(
            &DataKey::PendingAdmin,
            &PendingAdminProposal {
                proposed: proposed.clone(),
                proposed_at_ledger: env.ledger().sequence(),
            },
        );

        env.events().publish(
            (symbol_short!("admin"), Symbol::new(env, "proposed")),
            (admin, proposed.clone(), env.ledger().timestamp()),
        );
        true
    }

    /// Internal: accept a pending admin proposal, enforcing the timelock.
    pub(crate) fn accept_governance_admin_impl(env: Env) -> bool {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&crate::DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::NotInitialized);
        }

        let pending: Option<PendingAdminProposal> =
            env.storage().persistent().get(&DataKey::PendingAdmin);
        if pending.is_none() {
            env.panic_with_error(crate::Error::InvalidState);
        }
        let proposal = pending.unwrap();

        // Enforce treasury rotation timelock: acceptance is only allowed after
        // ADMIN_ROTATION_MIN_DELAY_LEDGERS have elapsed since the proposal.
        let elapsed = env
            .ledger()
            .sequence()
            .saturating_sub(proposal.proposed_at_ledger);
        if elapsed < ADMIN_ROTATION_MIN_DELAY_LEDGERS {
            env.panic_with_error(EscrowError::TimelockNotElapsed);
        }

        let pending_admin = proposal.proposed;
        pending_admin.require_auth();

        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(crate::Error::NotInitialized));

        env.storage()
            .persistent()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().persistent().remove(&DataKey::PendingAdmin);

        env.events().publish(
            (symbol_short!("admin"), Symbol::new(env, "accepted")),
            (old_admin, pending_admin.clone(), env.ledger().timestamp()),
        );
        true
    }

    /// Internal: return the currently pending admin address, if any.
    pub(crate) fn get_pending_governance_admin_impl(env: Env) -> Option<Address> {
        let proposal: Option<PendingAdminProposal> =
            env.storage().persistent().get(&DataKey::PendingAdmin);
        proposal.map(|p| p.proposed)
    }

    /// Internal: return the current admin address.
    pub(crate) fn get_governance_admin_impl(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Admin)
    }

    /// Set both governance parameters at once and update the readiness checklist.
    pub fn set_governed_params(
        env: Env,
        admin: Address,
        protocol_fee_bps: u32,
        max_escrow_total_stroops: i128,
    ) -> bool {
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&crate::DataKey::Initialized)
            .unwrap_or(false)
        {
            env.panic_with_error(EscrowError::NotInitialized);
        }

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));

        if admin != stored_admin {
            env.panic_with_error(EscrowError::UnauthorizedRole);
        }
        admin.require_auth();

        if protocol_fee_bps > 10_000 {
            env.panic_with_error(EscrowError::InvalidProtocolParameters);
        }

        let params = GovernedParameters {
            protocol_fee_bps,
            max_escrow_total_stroops,
        };
        env.storage()
            .persistent()
            .set(&DataKey::GovernedParameters, &params);

        let mut checklist: ReadinessChecklist = env
            .storage()
            .persistent()
            .get(&DataKey::ReadinessChecklist)
            .unwrap_or_default();
        checklist.governed_params_set = true;
        env.storage()
            .persistent()
            .set(&DataKey::ReadinessChecklist, &checklist);

        true
    }

    /// Retrieve the current governed parameters.
    pub fn get_governed_parameters(env: Env) -> Option<GovernedParameters> {
        env.storage().persistent().get(&DataKey::GovernedParameters)
    }
}
