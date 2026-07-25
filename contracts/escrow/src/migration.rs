use crate::ttl::{read_if_live, remove_transient, store_with_ttl, PENDING_MIGRATION_TTL_LEDGERS};
use crate::{Contract, ContractStatus, DataKey, Escrow, EscrowError};
use soroban_sdk::{contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingClientMigration {
    pub current_client: Address,
    pub proposed_client: Address,
    pub requested_at_ledger: u32,
    pub expires_at_ledger: u32,
}

impl Escrow {
    pub(crate) fn pending_migration_key(contract_id: u32) -> DataKey {
        DataKey::PendingClientMigration(contract_id)
    }

    pub(crate) fn load_contract(env: &Env, contract_id: u32) -> Contract {
        env.storage()
            .persistent()
            .get::<_, Contract>(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound))
    }

    pub(crate) fn require_migration_allowed(env: &Env, status: ContractStatus) {
        if matches!(
            status,
            ContractStatus::Completed
                | ContractStatus::Cancelled
                | ContractStatus::Refunded
                | ContractStatus::Disputed
        ) {
            env.panic_with_error(EscrowError::InvalidStatusTransition);
        }
    }

    pub(crate) fn pending_migration_exists(env: &Env, contract_id: u32) -> bool {
        read_if_live::<_, PendingClientMigration>(env, &Self::pending_migration_key(contract_id))
            .is_some()
    }
}

/// Propose a client migration for an existing contract.
///
/// The current client must authorize the call. The proposed client address
/// must not be the freelancer or the current client. The pending migration
/// is stored in temporary storage with TTL.
pub fn propose_client_migration_impl(
    env: &Env,
    contract_id: u32,
    current_client: Address,
    new_client: Address,
) -> bool {
    Escrow::require_not_paused(&env);
    current_client.require_auth();

    let contract = Escrow::load_contract(&env, contract_id);
    Escrow::require_not_finalized(&env, contract_id);
    if current_client != contract.client {
        env.panic_with_error(EscrowError::UnauthorizedRole);
    }
    if new_client == contract.client || new_client == contract.freelancer {
        env.panic_with_error(EscrowError::InvalidParticipant);
    }
    Escrow::require_migration_allowed(&env, contract.status);
    if Escrow::pending_migration_exists(&env, contract_id) {
        env.panic_with_error(EscrowError::InvalidState);
    }

    let requested_at = env.ledger().sequence();
    let expires_at = requested_at.saturating_add(PENDING_MIGRATION_TTL_LEDGERS);
    let pending = PendingClientMigration {
        current_client: current_client.clone(),
        proposed_client: new_client.clone(),
        requested_at_ledger: requested_at,
        expires_at_ledger: expires_at,
    };
    store_with_ttl(
        &env,
        &Escrow::pending_migration_key(contract_id),
        &pending,
        PENDING_MIGRATION_TTL_LEDGERS,
    );

    env.events().publish(
        (Symbol::new(&env, "client_migration_proposed"), contract_id),
        (current_client, new_client, requested_at),
    );
    true
}

/// Accept a live pending client migration and update the contract.
pub fn accept_client_migration_impl(env: &Env, contract_id: u32, new_client: Address) -> bool {
    Escrow::require_not_paused(&env);
    new_client.require_auth();

    let mut contract = Escrow::load_contract(&env, contract_id);
    Escrow::require_not_finalized(&env, contract_id);
    Escrow::require_migration_allowed(&env, contract.status);

    let key = Escrow::pending_migration_key(contract_id);
    let pending: PendingClientMigration =
        read_if_live(&env, &key).unwrap_or_else(|| env.panic_with_error(EscrowError::InvalidState));

    if pending.proposed_client != new_client {
        env.panic_with_error(EscrowError::UnauthorizedRole);
    }
    if pending.current_client != contract.client {
        env.panic_with_error(EscrowError::InvalidState);
    }

    contract.client = new_client.clone();
    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), &contract);
    remove_transient(&env, &key);

    env.events().publish(
        (Symbol::new(&env, "client_migration_accepted"), contract_id),
        (pending.current_client, new_client, env.ledger().timestamp()),
    );
    true
}

/// Return true if a live pending client migration exists.
pub fn has_pending_client_migration_impl(env: &Env, contract_id: u32) -> bool {
    Escrow::pending_migration_exists(&env, contract_id)
}

/// Return the live pending client migration record.
pub fn get_pending_client_migration_impl(env: &Env, contract_id: u32) -> PendingClientMigration {
    read_if_live(&env, &Escrow::pending_migration_key(contract_id))
        .unwrap_or_else(|| env.panic_with_error(EscrowError::InvalidState))
}
