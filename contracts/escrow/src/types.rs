use soroban_sdk::{
    contracterror, contracttype, Address, ConversionError, Env, IntoVal, String, Symbol,
    TryFromVal, Val, Vec,
};

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

/// Lightweight milestone entry returned by the paginated milestones view.
///
/// Carries only the fields needed for a UI listing: zero-based `index`,
/// a compact `status` code, and the milestone `amount` in stroops.
///
/// Status codes:
/// - `0` - Pending (not yet released or refunded)
/// - `1` - Released
/// - `2` - Refunded
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MilestoneEntry {
    pub index: u32,
    pub status: u32,
    pub amount: i128,
}

/// Lightweight contract entry returned by the paginated contracts view.
///
/// Carries only the fields needed for a UI listing: the contract `id`, a
/// numeric `status` code (the `ContractStatus` discriminant), and the
/// escrow's `funded_amount` / `released_amount` in stroops.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractEntry {
    pub id: u32,
    pub status: u32,
    pub funded_amount: i128,
    pub released_amount: i128,
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

/// Typed storage key for contract-owned entries that previously used ad-hoc
/// tuple keys such as `(DataKey::Contract(id), Symbol("milestones"))`.
///
/// The variants are intentionally narrow and match the storage shapes already
/// used by the escrow contract so the public behavior stays unchanged while the
/// call sites become clearer and more type-safe.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageKey {
    Contract(u32),
    ContractMilestones(u32),
    MilestoneApprovals(u32, u32),
    Finalization(u32),
    PendingClientMigration(u32),
}

impl StorageKey {
    pub fn contract(contract_id: u32) -> Self {
        Self::Contract(contract_id)
    }

    pub fn contract_milestones(contract_id: u32) -> Self {
        Self::ContractMilestones(contract_id)
    }

    pub fn milestone_approvals(contract_id: u32, milestone_index: u32) -> Self {
        Self::MilestoneApprovals(contract_id, milestone_index)
    }

    pub fn finalization(contract_id: u32) -> Self {
        Self::Finalization(contract_id)
    }

    pub fn pending_client_migration(contract_id: u32) -> Self {
        Self::PendingClientMigration(contract_id)
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    // Admin / pause / emergency
    Initialized,
    Admin,
    Paused,
    Emergency,
    StorageVersion,
    // Contract storage
    Contract(u32),
    NextContractId,
    MilestoneApprovals(u32, u32),
    // Reputation
    ReputationIssued(u32),
    PendingReputationCredits(ReputationKey),
    Reputation(ReputationKey),
    ReputationComment(u32),
    // Client migration
    PendingClientMigration(u32),
    // Settlement token
    SettlementToken,
    // Finalization
    Finalization(u32),
    // Protocol / governance
    GovernanceAdmin,
    PendingGovernanceAdmin,
    ProtocolParameters,
    ProtocolFeeBps,
    PendingAdmin,
    AccumulatedProtocolFees,
    GovernedParameters,
    ReadinessChecklist,
    // Configurable limits
    MaxMilestones,
    MaxEscrowStroops,
    // Finalization
    Finalization(u32),
    // Settlement token
    SettlementToken,
    Finalization(u32),
}

/// Typed key for milestone approval entries that include arbiter approval state.
///
/// This intentionally maps to the existing `DataKey::MilestoneApprovals`
/// variant so persisted storage layout and ABI remain unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArbiterApprovalKey {
    pub contract_id: u32,
    pub milestone_index: u32,
}

impl ArbiterApprovalKey {
    pub const fn new(contract_id: u32, milestone_index: u32) -> Self {
        Self {
            contract_id,
            milestone_index,
        }
    }
}

impl From<ArbiterApprovalKey> for DataKey {
    fn from(key: ArbiterApprovalKey) -> Self {
        DataKey::MilestoneApprovals(key.contract_id, key.milestone_index)
    }
}

/// Canonical contract error type for all entrypoint-facing errors.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    IndexOutOfBounds = 3,
    AlreadyReleased = 4,
    EmptyRefundRequest = 6,
    DuplicateMilestoneInRefund = 7,
    AlreadyRefunded = 8,
    InsufficientFunds = 9,
    ContractNotFound = 10,
    UnauthorizedRole = 11,
    MissingArbiter = 12,
    InvalidArbiter = 13,
    InvalidParticipants = 14,
    AmountMustBePositive = 15,
    InvalidState = 16,
    MilestoneAlreadyReleased = 17,
    AlreadyApproved = 18,
    InsufficientApprovals = 20,
    FreelancerMismatch = 21,
    InvalidRating = 22,
    ReputationAlreadyIssued = 23,
    EmptyMilestones = 25,
    InvalidMilestoneAmount = 26,
    ContractIdCollision = 27,
    ContractIdOverflow = 28,
    EmptyComment = 29,
    CommentTooLong = 30,
    InvalidParticipant = 31,
    InvalidDepositAmount = 32,
    InvalidMilestone = 33,
    AlreadyInitialized = 34,
    InsufficientAccumulatedFees = 35,
    NotInitialized = 36,
    ContractPaused = 37,
    EmergencyActive = 38,
    SelfRating = 39,
    NotCompleted = 40,
    InvalidStatusTransition = 41,
    ArbiterRequired = 42,
    InvalidDisputeSplit = 43,
    AccountingInvariantViolated = 44,
    PotentialOverflow = 45,
    AlreadyFinalized = 46,
    AlreadyCancelled = 50,
    EvidenceTooLong = 47,
    TimelockNotElapsed = 48,
    InvalidProtocolParameters = 49,
    EscrowCapExceeded = 51,
    SettlementTokenNotConfigured = 52,
    MilestoneNotOverdue = 53,
    /// The contract ID is out of valid bounds.
    InvalidContractId = 54,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadinessChecklist {
    pub initialized: bool,
    pub governed_params_set: bool,
    pub emergency_controls_enabled: bool,
}

impl Default for ReadinessChecklist {
    fn default() -> Self {
        ReadinessChecklist {
            initialized: false,
            governed_params_set: false,
            emergency_controls_enabled: false,
        }
    }
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

/// A single contract creation request within a batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractItem {
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Option<Address>,
    pub milestones: Vec<i128>,
    pub release_authorization: ReleaseAuthorization,
}

/// The result for a single item in a batch creation call.
///
/// On success, `contract_id` holds the assigned ID. On failure, `error_code`
/// holds the Soroban error code that would have been raised.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchContractResult {
    pub index: u32,
    pub contract_id: Option<u32>,
    pub error_code: Option<u32>,
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