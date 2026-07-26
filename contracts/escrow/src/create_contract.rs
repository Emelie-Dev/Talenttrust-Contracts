pub use crate::Escrow;
use crate::{
    amount_validation, ttl, Contract, ContractStatus, DataKey, Error, Escrow, EscrowArgs,
    EscrowClient, EscrowError, Milestone, ReleaseAuthorization, MAX_MILESTONES,
};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Vec};

pub fn execute_create_contract(
    env: Env,
    client: Address,
    freelancer: Address,
    arbiter: Option<Address>,
    milestones: Vec<i128>,
    release_authorization: ReleaseAuthorization,
) -> u32 {
        // Reject state-changing calls while paused or in emergency mode so every
        // mutating entrypoint halts uniformly. Runs before auth. See
        // finalize.rs::require_not_paused.
        crate::Escrow::require_not_paused(&env);

        client.require_auth();

        if client == freelancer {
            env.panic_with_error(EscrowError::InvalidParticipant);
        }

        match release_authorization {
            ReleaseAuthorization::ArbiterOnly | ReleaseAuthorization::ClientAndArbiter
                if arbiter.is_none() =>
            {
                env.panic_with_error(EscrowError::MissingArbiter);
            }
            _ => {}
        }

        if let Some(ref arb) = arbiter {
            if arb == &client || arb == &freelancer {
                env.panic_with_error(EscrowError::InvalidArbiter);
            }
        }

        if milestones.is_empty() {
            env.panic_with_error(EscrowError::EmptyMilestones);
        }

        if milestones.len() > MAX_MILESTONES {
            env.panic_with_error(EscrowError::TooManyMilestones);
        }

        // Retrieve governed parameters for total escrow cap; returns defaults
        // (i128::MAX) when `set_governed_params` has never been called.
        let max_total = Self::get_governed_parameters(env.clone()).max_escrow_total_stroops;

        let mut native_milestones = [0_i128; MAX_MILESTONES as usize];
        let len = milestones.len() as usize;
        for i in 0..len {
            native_milestones[i] = milestones.get(i as u32).unwrap();
        }
        match amount_validation::validate_milestone_amounts(&native_milestones[..len], max_total) {
            Ok(_) => (),
            Err(err) => match err {
                EscrowError::InvalidMilestoneAmount => {
                    env.panic_with_error(EscrowError::InvalidMilestoneAmount)
                }
                EscrowError::TotalCapExceeded => {
                    env.panic_with_error(EscrowError::TotalCapExceeded)
                }
                _ => env.panic_with_error(EscrowError::InvalidMilestoneAmount),
            },
        }

        ttl::extend_next_contract_id_ttl(&env);

        let id = next_contract_id(&env);

        let freelancer_addr = freelancer.clone();

        // Construct the contract with all required fields, initialising accounting
        // counters to zero and reputation_issued to false.
        let contract = Contract {
            client: client.clone(),
            freelancer: freelancer.clone(),
            arbiter,
            status: ContractStatus::Created,
            total_deposited: 0,
            funded_amount: 0,
            released_amount: 0,
            refunded_amount: 0,
            release_authorization,
            reputation_issued: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Contract(id), &contract);
        // New contracts are always written in the current layout, so stamp the
        // schema version marker now and skip the migration-on-read path.
        env.storage().persistent().set(
            &DataKey::ContractSchemaVersion(id),
            &CONTRACT_STORAGE_SCHEMA_VERSION,
        );

        // Build and persist the milestone vector.
        let mut milestone_vec: Vec<Milestone> = Vec::new(&env);
        for amount in milestones.iter() {
            milestone_vec.push_back(Milestone {
                amount,
                funded_amount: 0,
                released: false,
                refunded: false,
                work_evidence: None,
                refunded_amount: 0,
                deadline: None,
            });
        }
        env.storage()
            .persistent()
            .set(&DataKey::Milestones(id), &milestone_vec);

        let next_id = id
            .checked_add(1)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractIdOverflow));
        env.storage()
            .persistent()
            .set(&DataKey::NextContractId, &next_id);

        env.events().publish(
            (symbol_short!("created"), id),
            (client, freelancer_addr, env.ledger().timestamp()),
        );

    id
}

/// Returns the next available contract ID and asserts it is not already occupied.
///
/// # Errors
/// * `ContractIdCollision` - If the allocated id slot is already occupied
pub(crate) fn next_contract_id(env: &Env) -> u32 {
    let id: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::NextContractId)
        .unwrap_or(1);

    if env
        .storage()
        .persistent()
        .get::<_, Contract>(&DataKey::Contract(id))
        .is_some()
    {
        env.panic_with_error(EscrowError::ContractIdCollision);
    }

    id
}
