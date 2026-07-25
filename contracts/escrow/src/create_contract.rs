use crate::{ttl, Contract, ContractStatus, DataKey, Error, Milestone, ReleaseAuthorization};
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

/// Creates a new escrow contract with the specified client, freelancer, and milestone amounts.
///
/// # Arguments
/// * `env` - The contract environment
/// * `client` - The address of the client funding the contract
/// * `freelancer` - The address of the freelancer performing the work
/// * `arbiter` - Optional arbiter address for dispute resolution
/// * `milestones` - Vector of milestone amounts (in stroops)
/// * `release_authorization` - Authorization mode for milestone releases
/// * `deadlines` - Optional per-milestone deadlines (Unix timestamps in seconds)
///
/// # Returns
/// The unique contract ID
///
/// # Errors
/// * `InvalidParticipants` - If client and freelancer are the same address
/// * `EmptyMilestones` - If no milestones are provided
/// * `InvalidMilestoneAmount` - If any milestone amount is <= 0
/// * `MissingArbiter` - If arbiter is required but not provided
/// * `InvalidArbiter` - If arbiter is same as client or freelancer
/// * `ContractIdOverflow` - If the next id would exceed `u32::MAX`
/// * `ContractIdCollision` - If the allocated id slot is already occupied
pub fn create_contract_impl(
    env: &Env,
    client: Address,
    freelancer: Address,
    arbiter: Option<Address>,
    milestones: Vec<i128>,
    release_authorization: ReleaseAuthorization,
    deadlines: Option<Vec<u64>>,
) -> u32 {
    client.require_auth();

    if client == freelancer {
        env.panic_with_error(Error::InvalidParticipants);
    }

    match release_authorization {
        ReleaseAuthorization::ArbiterOnly | ReleaseAuthorization::ClientAndArbiter
            if arbiter.is_none() =>
        {
            env.panic_with_error(Error::MissingArbiter);
        }
        _ => {}
    }

    if let Some(ref arb) = arbiter {
        if arb == &client || arb == &freelancer {
            env.panic_with_error(Error::InvalidArbiter);
        }
    }

    if milestones.is_empty() {
        env.panic_with_error(Error::EmptyMilestones);
    }

    for amount in milestones.iter() {
        if amount <= 0 {
            env.panic_with_error(Error::InvalidMilestoneAmount);
        }
    }

    // Validate deadline count matches milestone count
    if let Some(ref deadlines_vec) = deadlines {
        if deadlines_vec.len() != milestones.len() {
            env.panic_with_error(Error::InvalidMilestoneAmount);
        }
    }

    let id = next_contract_id(&env);

    ttl::extend_next_contract_id_ttl(&env);

    let freelancer_addr = freelancer.clone();
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
    };
    env.storage()
        .persistent()
        .set(&DataKey::Contract(id), &contract);

    let mut milestone_vec: Vec<Milestone> = Vec::new(&env);
    for (i, amount) in milestones.iter().enumerate() {
        let deadline = deadlines.as_ref().and_then(|d| d.get(i as u32));
        milestone_vec.push_back(Milestone {
            amount,
            funded_amount: 0,
            released: false,
            refunded: false,
            work_evidence: None,
            refunded_amount: 0,
            deadline,
        });
    }
    let milestone_key = Symbol::new(&env, "milestones");
    env.storage()
        .persistent()
        .set(&(DataKey::Contract(id), milestone_key), &milestone_vec);

    env.storage()
        .persistent()
        .set(&DataKey::NextContractId, &(id + 1));

    env.events().publish(
        (symbol_short!("created"), id),
        (client, freelancer_addr, env.ledger().timestamp()),
    );

    id
}

/// Returns the next contract id after verifying the slot is unused.
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
        env.panic_with_error(Error::ContractIdCollision);
    }

    id
}
