//! Governance and protocol-configuration entrypoints.
//!
//! This module owns admin-controlled persistent configuration:
//! `DataKey::Admin` for authorization, `ProtocolFeeBps` for release fees,
//! `GovernedParameters` for escrow caps, `ReadinessChecklist` for deployment
//! readiness state, and `PendingAdmin` for two-step admin rotation proposals.
//! Money movement for protocol-fee withdrawal remains in the crate root because
//! it performs settlement-token transfers.

use crate::ttl::ADMIN_ROTATION_MIN_DELAY_LEDGERS;
use crate::{
    DataKey, DisputeConfig, Error, Escrow, EscrowArgs, EscrowClient, GovernedParameters,
    PendingAdminProposal, ReadinessChecklist,
};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol};

#[contractimpl]
impl Escrow {
    /// Set the protocol fee in basis points.
    ///
    /// Admin-gated: the stored admin (under [`DataKey::Admin`]) must authorize
    /// the call and the contract must be initialized.
    ///
    /// `new_bps` must be `≤ 10_000` (100%). The fee takes effect immediately for
    /// the next `release_milestone` call. Values above 10_000 are rejected with
    /// `InvalidProtocolParameters` because a fee exceeding 100% would make every
    /// milestone release net negative for the freelancer.
    ///
    /// See [`docs/escrow/protocol-fees.md`](../../../docs/escrow/protocol-fees.md) for
    /// the basis-point model, fee formula, accrual storage, and withdrawal flow.
    ///
    /// # Errors
    /// * `NotInitialized` - if `initialize` has not been called
    /// * `UnauthorizedRole` - if the caller is not the stored admin
    /// * `InvalidProtocolParameters` - if `new_bps > 10_000`
    ///
    /// # Events
    /// `(Symbol("protocol_fee_bps"),)` → `(old_bps, new_bps, admin, timestamp)`
    pub(crate) fn set_protocol_fee_bps_impl(env: Env, new_bps: u32) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
        admin.require_auth();

        // Reject any fee above 100 % (10_000 bps). A fee > 100 % would make every
        // milestone release impossible — the net payout would be negative.
        if new_bps > 10_000 {
            env.panic_with_error(Error::InvalidProtocolParameters);
        }

        let old_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ProtocolFeeBps)
            .unwrap_or(0u32);
        env.storage()
            .persistent()
            .set(&DataKey::ProtocolFeeBps, &new_bps);

        env.events().publish(
            (Symbol::new(&env, "protocol_fee_bps"),),
            (old_bps, new_bps, admin.clone(), env.ledger().timestamp()),
        );
        true
    }

    /// Returns the current protocol fee in basis points.
    pub(crate) fn get_protocol_fee_bps_impl(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get::<_, u32>(&DataKey::ProtocolFeeBps)
            .unwrap_or(0)
    }

    /// Set the maximum events limit for queries or indexing.
    ///
    /// Admin-gated: the stored admin (under [`DataKey::Admin`]) must authorize
    /// the call and the contract must be initialized.
    ///
    /// `new_limit` must be within safe bounds (e.g., > 0 and <= 1000).
    ///
    /// # Events
    /// `(Symbol("events_limit"),)` → `(old_limit, new_limit, admin, timestamp)`
    pub fn set_events_limit(env: Env, new_limit: u32) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(Error::NotInitialized));
        admin.require_auth();

        if new_limit == 0 || new_limit > 1000 {
            env.panic_with_error(Error::InvalidEventsLimit);
        }

        let old_limit: u32 = Self::get_events_limit(env.clone());
        
        env.storage()
            .persistent()
            .set(&DataKey::EventsLimit, &new_limit);

        env.events().publish(
            (Symbol::new(&env, "events_limit"),),
            (old_limit, new_limit, admin, env.ledger().timestamp()),
        );
        true
    }

    /// Returns the current events limit. Default is 100.
    pub fn get_events_limit(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get::<_, u32>(&DataKey::EventsLimit)
            .unwrap_or(100) // Default preserves current behaviour
    }

    // ── Two-step admin transfer ───────────────────────────────────────────────

    /// Propose a new governance admin. Stores the proposal with a timelock.
    ///
    /// # Events
    /// `(symbol_short!("admin"), Symbol("proposed"))` → `(admin, proposed, timestamp)`
    pub fn propose_governance_admin(env: Env, proposed: Address) -> bool {
        Self::require_initialized(&env);

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        admin.require_auth();

        env.storage().persistent().set(
            &DataKey::PendingAdmin,
            &PendingAdminProposal {
                proposed: proposed.clone(),
                proposed_at_ledger: env.ledger().sequence(),
            },
        );

        env.events().publish(
            (symbol_short!("admin"), Symbol::new(&env, "proposed")),
            (admin, proposed.clone(), env.ledger().timestamp()),
        );
        true
    }

    /// Accept a pending admin proposal, enforcing the timelock.
    ///
    /// # Events
    /// `(symbol_short!("admin"), Symbol("accepted"))` → `(old_admin, new_admin, timestamp)`
    pub fn accept_governance_admin(env: Env) -> bool {
        Self::require_initialized(&env);

        let pending: PendingAdminProposal = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::InvalidState));

        let elapsed = env
            .ledger()
            .sequence()
            .saturating_sub(pending.proposed_at_ledger);
        if elapsed < ADMIN_ROTATION_MIN_DELAY_LEDGERS {
            env.panic_with_error(EscrowError::TimelockNotElapsed);
        }

        let pending_admin = pending.proposed;
        pending_admin.require_auth();

        let old_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

        env.storage()
            .persistent()
            .set(&DataKey::Admin, &pending_admin);
        env.storage().persistent().remove(&DataKey::PendingAdmin);

        env.events().publish(
            (symbol_short!("admin"), Symbol::new(&env, "accepted")),
            (old_admin, pending_admin.clone(), env.ledger().timestamp()),
        );
        true
    }

    /// Cancel a pending governance admin proposal, aborting a two-step transfer.
    ///
    /// Only the current admin (the address stored under [`DataKey::Admin`]) may
    /// cancel, and the contract must be initialized. On success the pending
    /// proposal is removed so the previously proposed address can no longer call
    /// [`Escrow::accept_governance_admin`] — a subsequent accept panics with
    /// [`Error::InvalidState`].
    ///
    /// # Errors
    /// * [`Error::NotInitialized`] — `initialize` has not been called.
    /// * [`Error::InvalidState`] — there is no pending proposal to cancel.
    ///
    /// # Events
    /// `(symbol_short!("admin"), Symbol("cancelled"))` → `(admin, cancelled_proposal, timestamp)`
    pub fn cancel_governance_admin_proposal(env: Env) -> bool {
        Self::require_initialized(&env);

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));
        admin.require_auth();

        let pending: PendingAdminProposal = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdmin)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::InvalidState));

        env.storage().persistent().remove(&DataKey::PendingAdmin);

        env.events().publish(
            (symbol_short!("admin"), Symbol::new(&env, "cancelled")),
            (admin, pending.proposed, env.ledger().timestamp()),
        );
        true
    }

    /// Return the currently pending admin address, if any.
    pub fn get_pending_governance_admin(env: Env) -> Option<Address> {
        let proposal: Option<PendingAdminProposal> =
            env.storage().persistent().get(&DataKey::PendingAdmin);
        proposal.map(|p| p.proposed)
    }

    /// Internal: return the current admin address.
    pub(crate) fn get_governance_admin_impl(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::Admin)
    }

    /// Set both governance parameters at once and update the readiness checklist.
    ///
    /// Sets `protocol_fee_bps` (must be `≤ MAX_FEE_BPS`) and `max_escrow_total_stroops`
    /// atomically. Also flips `ReadinessChecklist::governed_params_set` to `true`.
    ///
    /// See [`docs/escrow/protocol-fees.md`](../../../docs/escrow/protocol-fees.md) for
    /// the full basis-point model and fee lifecycle.
    pub(crate) fn set_governed_params_impl(
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
        env.storage()
            .persistent()
            .set(&DataKey::ProtocolFeeBps, &protocol_fee_bps);

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
    ///
    /// Returns a [`GovernedParameters`] value populated from storage when
    /// `set_governed_params` has been called, or from the same default
    /// constants the enforcement code uses when storage is empty.
    ///
    /// The defaults match what the enforcement paths apply when no
    /// governance parameters have been configured:
    /// - `protocol_fee_bps`: `0` (no protocol fee withheld on release)
    /// - `max_escrow_total_stroops`: `i128::MAX` (no effective cap)
    ///
    /// Callers that need to distinguish "governance has not written" from
    /// "governance wrote values that happen to match defaults" should use
    /// [`is_governed_params_set`](Self::is_governed_params_set) which
    /// checks whether `set_governed_params` has ever succeeded.
    pub fn get_governed_parameters(env: Env) -> GovernedParameters {
        env.storage()
            .persistent()
            .get::<_, GovernedParameters>(&DataKey::GovernedParameters)
            .unwrap_or(GovernedParameters {
                protocol_fee_bps: 0,
                max_escrow_total_stroops: i128::MAX,
            })
    }

    /// Returns `true` if `set_governed_params` has ever been called
    /// successfully, `false` otherwise.
    ///
    /// This lets integrators distinguish between "governance wrote defaults"
    /// and "governance has not written anything yet". The underlying flag
    /// is the `governed_params_set` field of the [`ReadinessChecklist`]
    /// stored under [`DataKey::ReadinessChecklist`].
    pub fn is_governed_params_set(env: Env) -> bool {
        env.storage()
            .persistent()
            .get::<_, crate::ReadinessChecklist>(&DataKey::ReadinessChecklist)
            .map(|c| c.governed_params_set)
            .unwrap_or(false)
    }

    /// Set the admin-configurable per-contract storage limit (in bytes).
    ///
    /// Admin-gated: the stored admin (under [`DataKey::Admin`]) must authorize the
    /// call and the contract must be initialized.
    ///
    /// The `new_limit` must be within the range `[MIN_STORAGE_LIMIT,
    /// MAX_STORAGE_LIMIT]` (1 – 1 000 000 bytes inclusive).  Values outside this
    /// range are rejected with [`Error::StorageLimitOutOfRange`].  The default
    /// value — applied whenever no admin has overridden the limit — is
    /// `DEFAULT_STORAGE_LIMIT` (64 KB = 65 536 bytes), which preserves the
    /// behaviour that existed before this entrypoint was introduced.
    ///
    /// # Errors
    /// * [`Error::NotInitialized`] — `initialize` has not been called.
    /// * [`Error::UnauthorizedRole`] — `admin` is not the stored admin.
    /// * [`Error::StorageLimitOutOfRange`] — `new_limit < MIN_STORAGE_LIMIT` or
    ///   `new_limit > MAX_STORAGE_LIMIT`.
    ///
    /// # Events
    /// `(Symbol("storage_limit"),)` → `(old_limit, new_limit, admin, timestamp)`
    pub fn set_storage_limit(env: Env, admin: Address, new_limit: u32) -> bool {
        Self::require_initialized(&env);

        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(Error::NotInitialized));

        if admin != stored_admin {
            env.panic_with_error(Error::UnauthorizedRole);
        }
        admin.require_auth();

        if new_limit < crate::MIN_STORAGE_LIMIT || new_limit > crate::MAX_STORAGE_LIMIT {
            env.panic_with_error(Error::StorageLimitOutOfRange);
        }

        let old_limit: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StorageLimit)
            .unwrap_or(crate::DEFAULT_STORAGE_LIMIT);

        env.storage()
            .persistent()
            .set(&DataKey::StorageLimit, &new_limit);

        env.events().publish(
            (Symbol::new(&env, "storage_limit"),),
            (old_limit, new_limit, admin, env.ledger().timestamp()),
        );

        true
    }

    /// Return the current per-contract storage limit in bytes.
    ///
    /// Returns [`DEFAULT_STORAGE_LIMIT`] when no admin has overridden the value.
    /// Read-only and auth-free.
    pub fn get_storage_limit(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::StorageLimit)
            .unwrap_or(crate::DEFAULT_STORAGE_LIMIT)
    }

    /// Read-only view returning the disputes configuration values without mutating storage.
    /// Returns sensible default values before initialization or if unconfigured.
    pub fn get_disputes_config(env: Env) -> DisputeConfig {
        env.storage()
            .persistent()
            .get(&DataKey::DisputeConfigKey)
            .unwrap_or_default()
    }

    /// Read-only view alias returning disputes configuration values without mutating storage.
    pub fn get_dispute_config(env: Env) -> DisputeConfig {
        Self::get_disputes_config(env)
    }

    /// Admin-gated entrypoint to update disputes configuration.
    pub fn set_disputes_config(
        env: Env,
        freelancer_share_bps: u32,
        client_share_bps: u32,
    ) -> bool {
        Self::require_initialized(&env);
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(Error::NotInitialized));
        admin.require_auth();

        let config = DisputeConfig {
            partial_refund_freelancer_bps: freelancer_share_bps,
            partial_refund_client_bps: client_share_bps,
        };
        env.storage()
            .persistent()
            .set(&DataKey::DisputeConfigKey, &config);
        true
    }
}
