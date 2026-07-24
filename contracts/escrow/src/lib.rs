#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractStatus {
    Created = 0,
    Funded = 1,
    Completed = 2,
    Disputed = 3,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Milestone {
    pub amount: i128,
    pub released: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowState {
    pub client: Address,
    pub freelancer: Address,
    pub milestones: Vec<Milestone>,
    pub status: ContractStatus,
}

#[contract]
pub struct Escrow;

#[contractimpl]
impl Escrow {
    /// Create a new escrow contract. Client and freelancer addresses are stored
    /// for access control. Milestones define payment amounts.
    pub fn create_contract(
        env: Env,
        client: Address,
        freelancer: Address,
        milestone_amounts: Vec<i128>,
    ) -> u32 {
        client.require_auth();

        let contract_id = 1u32;

        let mut milestones = Vec::new(&env);
        for amount in milestone_amounts {
            milestones.push_back(Milestone {
                amount,
                released: false,
            });
        }

        let state = EscrowState {
            client,
            freelancer,
            milestones,
            status: ContractStatus::Created,
        };

        env.storage().persistent().set(&contract_id, &state);
        contract_id
    }

    /// Deposit funds into escrow. Only the client may call this.
    pub fn deposit_funds(env: Env, contract_id: u32, amount: i128) -> bool {
        let mut state: EscrowState = env.storage().persistent().get(&contract_id).unwrap();
        state.client.require_auth();

        // In a real implementation, we would handle token transfers here
        state.status = ContractStatus::Funded;
        env.storage().persistent().set(&contract_id, &state);

        true
    }

    /// Release a milestone payment to the freelancer after verification.
    pub fn release_milestone(env: Env, contract_id: u32, milestone_id: u32) -> bool {
        let mut state: EscrowState = env.storage().persistent().get(&contract_id).unwrap();
        state.client.require_auth();

        let mut milestones = state.milestones;
        let mut milestone = milestones.get(milestone_id).unwrap();

        if !milestone.released {
            milestone.released = true;
            milestones.set(milestone_id, milestone.clone());

            // Emit milestone state change event
            Self::emit_milestone_state_change(&env, contract_id, milestone_id, milestone);

            // Check if all milestones are released
            let all_released = milestones.iter().all(|m| m.released);
            if all_released {
                state.status = ContractStatus::Completed;
            }

            state.milestones = milestones;
            env.storage().persistent().set(&contract_id, &state);
        }

        true
    }

    /// Issue a reputation credential for the freelancer after contract completion.
    pub fn issue_reputation(_env: Env, _freelancer: Address, _rating: i128) -> bool {
        // Reputation credential issuance.
        true
    }

    /// Hello-world style function for testing and CI.
    pub fn hello(_env: Env, to: Symbol) -> Symbol {
        to
    }

    /// Helper function to emit milestone state change event
    fn emit_milestone_state_change(
        env: &Env,
        contract_id: u32,
        milestone_id: u32,
        milestone: Milestone,
    ) {
        env.events().publish(
            (symbol_short!("ms_state"), contract_id, milestone_id),
            milestone,
        );
    }
}

#[cfg(test)]
mod test;
