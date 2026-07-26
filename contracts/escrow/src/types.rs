use soroban_sdk::{contracttype, Address, String, Vec};

// ── Indexer summary types ────────────────────────────────────────────────────

#[allow(dead_code)]
pub const CONTRACT_SUMMARY_SCHEMA_VERSION: u32 = 1;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneSummary {
    pub index: u32,
    pub amount: i128,
    pub released: bool,
    pub refunded: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneEntry {
    pub index: u32,
    pub status: u32,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractSummary {
    pub schema_version: u32,
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Option<Address>,
    pub status: ContractStatus,
    pub reputation_issued: bool,
    pub total_amount: i128,
    pub funded_amount: i128,
    pub released_amount: i128,
    pub refundable_balance: i128,
    pub released_milestone_count: u32,
    pub milestones: Vec<MilestoneSummary>,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractBounds {
    pub max_milestones: u32,
    pub max_single_milestone_stroops: i128,
    pub max_total_escrow_stroops: i128,
    pub max_fee_bps: u32,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    // Admin / pause / emergency
    Initialized,
    Admin,
    Paused,
    Emergency,
    // Contract storage
    Contract(u32),
    NextContractId,
    MilestoneApprovals(u32, u32),
    // Reputation
    ReputationIssued(u32),
    PendingReputationCredits(Address),
    Reputation(Address),
    ReputationComment(u32),
    // Client migration
    PendingClientMigration(u32),
    // Protocol / governance
    ProtocolFeeBps,
    PendingAdmin,
    AccumulatedProtocolFees,
    GovernedParameters,
    ReadinessChecklist,
    // Finalization
    Finalization(u32),
    // Settlement token
    SettlementToken,
    // State migration
    State,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractStatus {
    Created = 0,
    Accepted = 1,
    Funded = 2,
    Completed = 3,
    Disputed = 4,
    Cancelled = 5,
    Refunded = 6,
    PartiallyFunded = 7,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contract {
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Option<Address>,
    pub status: ContractStatus,
    pub total_deposited: i128,
    pub funded_amount: i128,
    pub released_amount: i128,
    pub refunded_amount: i128,
    pub release_authorization: ReleaseAuthorization,
    pub reputation_issued: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Milestone {
    pub amount: i128,
    pub funded_amount: i128,
    pub released: bool,
    pub refunded: bool,
    pub work_evidence: Option<String>,
    pub refunded_amount: i128,
    pub deadline: Option<u64>,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseAuthorization {
    ClientOnly = 0,
    ClientAndArbiter = 1,
    ArbiterOnly = 2,
    MultiSig = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneApprovals {
    pub client_approved: bool,
    pub freelancer_approved: bool,
    pub arbiter_approved: bool,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepositMode {
    ExactTotal = 0,
    Incremental = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ReadinessChecklist {
    pub initialized: bool,
    pub governed_params_set: bool,
    pub emergency_controls_enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovernedParameters {
    pub protocol_fee_bps: u32,
    pub max_escrow_total_stroops: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAdminProposal {
    pub proposed: Address,
    pub proposed_at_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct Reputation {
    pub completed_contracts: i128,
    pub total_rating: i128,
    pub last_rating: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationBatchItem {
    pub contract_id: u32,
    pub rating: u32,
    pub comment: String,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeSplit {
    pub client_amount: i128,
    pub freelancer_amount: i128,
}

pub type SplitAmounts = DisputeSplit;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisputeResolution {
    FullRefund,
    PartialRefund,
    FullPayout,
    Split(DisputeSplit),
}

impl DisputeResolution {
    pub fn code(&self) -> u32 {
        match self {
            Self::FullRefund => 0,
            Self::PartialRefund => 1,
            Self::FullPayout => 2,
            Self::Split(_) => 3,
        }
    }
}

// ── State Migration Types ────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateV1 {
    pub client: Address,
    pub freelancer: Address,
    pub milestones: Vec<i128>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateV2 {
    pub client: Address,
    pub freelancer: Address,
    pub milestones: Vec<i128>,
    pub status: ContractStatus,
}