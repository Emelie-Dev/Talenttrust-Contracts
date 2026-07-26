pub use crate::Escrow;
use crate::{
    amount_validation, ttl, Contract, ContractStatus, DataKey, Escrow, EscrowArgs, EscrowClient,
    EscrowError, GovernedParameters, Milestone, ReleaseAuthorization, Error, MAX_MILESTONES,
    amount_validation, ttl, Contract, ContractStatus, DataKey, Error, Escrow, EscrowArgs,
    EscrowClient, EscrowError, GovernedParameters, Milestone, ReleaseAuthorization,
    SimulateCreateContractOutcome, MAX_MILESTONES,
};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Symbol, Vec};

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

        // Require a bound settlement token before creating escrows.
        let bound_token = Self::read_settlement_token(&env)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::SettlementTokenNotConfigured));

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
            token: bound_token,
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
        for (idx, amount) in milestones.iter().enumerate() {
            milestone_vec.push_back(Milestone {
                amount,
                funded_amount: 0,
                released: false,
                refunded: false,
                work_evidence: None,
                refunded_amount: 0,
                deadline: None,
            });
            // Indexed event for off-chain milestone-history reconstruction.
            env.events().publish(
                (symbol_short!("mlstn_idx"), id, idx as u32),
                (amount, false, false, env.ledger().timestamp()),
            );
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

        // Emit arbiter assignment event so off-chain indexers can reconstruct
        // the full arbiter history from events alone.
        if let Some(ref arb) = contract.arbiter {
            env.events().publish(
                (symbol_short!("arbiter"), id),
                (None::<Address>, arb.clone(), env.ledger().timestamp()),
            );
        }

        id

    }

    /// Simulates contract creation without writing to storage or emitting events.
    ///
    /// This is a read-only variant of [`create_contract`](Self::create_contract) that
    /// performs all the same validation checks but returns the projected outcome without:
    /// - Mutating storage
    /// - Emitting events
    /// - Incrementing the contract ID counter
    ///
    /// The simulated contract ID is based on the current `NextContractId` value at the
    /// time of the call. If validation fails, the function panics with the same error
    /// as the real `create_contract` would.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `client` - The address of the client funding the contract
    /// * `freelancer` - The address of the freelancer performing the work
    /// * `arbiter` - Optional arbiter address for dispute resolution
    /// * `milestones` - Vector of milestone amounts (in stroops)
    /// * `release_authorization` - Authorization mode for milestone releases
    ///
    /// # Returns
    /// A [`SimulateCreateContractOutcome`] containing the projected contract details,
    /// including the simulated contract ID and all input parameters.
    ///
    /// # Errors
    /// Same as [`create_contract`](Self::create_contract):
    /// * `InvalidParticipant`   - If client and freelancer are the same address
    /// * `EmptyMilestones`      - If no milestones are provided
    /// * `InvalidMilestoneAmount` - If any milestone amount is <= 0
    /// * `MissingArbiter`       - If arbiter is required but not provided
    /// * `InvalidArbiter`       - If arbiter is same as client or freelancer
    /// * `TooManyMilestones`    - If the number of milestones exceeds `MAX_MILESTONES`
    /// * `TotalCapExceeded`     - If the sum of milestone amounts exceeds the governed cap
    ///
    /// # Notes
    /// - The simulated contract ID is **not** consumed; `create_contract` will still
    ///   use the same ID (or the next available one if contract creation occurs after simulation).
    /// - The `client` address **does not** require authorization for this read-only operation.
    /// - All participant validation (distinct client/freelancer, valid arbiter) is performed.
    /// - All milestone validation (non-empty, positive amounts, within cap) is performed.
    ///
    /// # Example
    /// ```ignore
    /// let outcome = escrow.simulate_create_contract(
    ///     &env,
    ///     &client,
    ///     &freelancer,
    ///     &None,
    ///     &milestones,
    ///     &ReleaseAuthorization::ClientOnly,
    /// );
    /// // outcome.contract_id is the ID that would be assigned
    /// // No storage has been modified
    /// ```
    pub fn simulate_create_contract(
        env: Env,
        client: Address,
        freelancer: Address,
        arbiter: Option<Address>,
        milestones: Vec<i128>,
        release_authorization: ReleaseAuthorization,
    ) -> SimulateCreateContractOutcome {
        // Validate that client and freelancer are distinct participants.
        if client == freelancer {
            env.panic_with_error(EscrowError::InvalidParticipant);
        }

        // Validate arbiter requirement based on release authorization mode.
        match release_authorization {
            ReleaseAuthorization::ArbiterOnly | ReleaseAuthorization::ClientAndArbiter
                if arbiter.is_none() =>
            {
                env.panic_with_error(EscrowError::MissingArbiter);
            }
            _ => {}
        }

        // Validate arbiter is distinct from both client and freelancer.
        if let Some(ref arb) = arbiter {
            if arb == &client || arb == &freelancer {
                env.panic_with_error(EscrowError::InvalidArbiter);
            }
        }

        // Validate at least one milestone is specified.
        if milestones.is_empty() {
            env.panic_with_error(EscrowError::EmptyMilestones);
        }

        // Enforce maximum number of milestones.
        if milestones.len() > MAX_MILESTONES {
            env.panic_with_error(EscrowError::TooManyMilestones);
        }

        // Retrieve governed parameters for total escrow cap; allow any total if unset.
        let max_total = env
            .storage()
            .persistent()
            .get::<_, GovernedParameters>(&DataKey::GovernedParameters)
            .map(|params| params.max_escrow_total_stroops)
            .unwrap_or(i128::MAX);

        // Validate milestone amounts and enforce the total cap via the canonical helper.
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

        // Get the next contract ID (read-only, no state mutation)
        let simulated_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextContractId)
            .unwrap_or(1);

        // Calculate total amount (sum of all milestones)
        let mut total_amount: i128 = 0;
        for milestone_amount in milestones.iter() {
            total_amount = total_amount
                .checked_add(milestone_amount)
                .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
        }

        SimulateCreateContractOutcome {
            contract_id: simulated_id,
            client,
            freelancer,
            arbiter,
            release_authorization,
            milestones,
            total_amount,
        }
    }
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
