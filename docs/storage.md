# Escrow storage model and invariants

This document describes the on-ledger storage used by the Soroban escrow
contract. It is intentionally tied to the current implementation:
[`DataKey`](../contracts/escrow/src/types.rs) defines the key schema,
[`ttl`](../contracts/escrow/src/ttl.rs) defines the retention policy, and the
entrypoints below are the authoritative read and write paths.

## Storage classes

The contract uses persistent and temporary Soroban storage. It does not keep
application records in instance storage.

| Class | Records | Retention rule |
| --- | --- | --- |
| Persistent | Configuration, escrow contracts, milestone vectors, accounting, reputation, governance, and finalization records | Contract and milestone helpers renew to 30 days when fewer than 7 days remain. Other persistent keys have the host's normal persistent lifetime unless their writer explicitly renews them. |
| Temporary | Outstanding milestone approvals and pending client migrations | Approvals live for 7 days; migrations live for 21 days. An expired or absent record is treated as unavailable. |

TTL values are ledger counts, using `17,280` ledgers per day. Temporary
entries are deliberately fail-closed: expiry cannot preserve an authorization
or migration request. See [`ttl.rs`](../contracts/escrow/src/ttl.rs) for the
constants and helpers.

## Key and value schema

All application keys are variants of `DataKey`, except the milestone vector,
which is a composite persistent key.

| Key | Storage | Value | Lifecycle / owner |
| --- | --- | --- | --- |
| `Initialized` | persistent | `bool` | Written once by `initialize`; gates lifecycle operations. |
| `Admin` | persistent | `Address` | Written by `initialize`; changed only through the two-step admin flow. |
| `SettlementToken` | persistent | `Address` | Written once by `bind_settlement_token`; identifies the SAC used for transfers. |
| `Paused`, `Emergency` | persistent | `bool` | Admin controls. A true value blocks protected mutations. |
| `NextContractId` | persistent | `u32` | Starts at 1 and is advanced by successful contract creation. |
| `Contract(id)` | persistent | `Contract` | Per-escrow participants, status, cumulative amounts, release mode, and reputation flag. |
| `(Contract(id), "milestones")` | persistent | `Vec<Milestone>` | The matching contract's ordered milestones, including per-milestone funding, release, refund, deadline, and evidence state. |
| `MilestoneApprovals(id, index)` | temporary | `MilestoneApprovals` | Live approval flags for one unreleased milestone. Removed when that milestone is settled. |
| `PendingClientMigration(id)` | temporary | `PendingClientMigration` | Proposed client replacement; currently exposed by migration helpers and cancellation. |
| `ProtocolFeeBps`, `AccumulatedProtocolFees` | persistent | `u32`, `i128` | Fee configuration and the fees retained during releases. |
| `PendingAdmin` | persistent | `PendingAdminProposal` | Candidate administrator and proposal ledger; removed on acceptance or cancellation. |
| `GovernedParameters` | persistent | `GovernedParameters` | Admin-configured fee and escrow cap used at creation. |
| `ReadinessChecklist` | persistent | `ReadinessChecklist` | Operational setup markers. |
| `ReputationIssued(id)` | persistent | `bool` | One-time issuance guard. |
| `PendingReputationCredits(address)` | persistent | `i128` | Credits accrued by completed releases for a freelancer. |
| `Reputation(address)` | persistent | `Reputation` | Aggregated completed-contract and rating record. |
| `ReputationComment(id)` | persistent | `String` | Comment associated with issued reputation. |
| `Finalization(id)` | persistent | `FinalizationRecord` | Immutable close snapshot; its presence blocks later contract-specific mutations. |

`MilestoneReleased`, `GovernanceAdmin`, `PendingGovernanceAdmin`, and
`ProtocolParameters` remain declared `DataKey` variants, but the current
implementation does not read or write them. In particular, release state is
not duplicated: `Milestone.released` in the milestone vector is the sole
source of truth.

## Invariants

1. **A contract and its milestone vector are a pair.** Successful
   `create_contract` writes both keys before advancing `NextContractId`.
   Readers treat a missing member of the pair as `ContractNotFound`.
2. **Contract IDs are unique and monotonic.** The counter begins at 1,
   checks its candidate slot for collision, uses checked addition, and is only
   advanced after the records have been stored. IDs are never reused.
3. **Milestone state is canonical and monotonic.** The ordered vector is the
   only record of release/refund flags and per-milestone funding. A release or
   refund rejects an already-settled milestone; a separate
   `MilestoneReleased` key must not be introduced as a second authority.
4. **Accounting is conserved.** For a contract,
   `refundable_balance = funded_amount - released_amount - refunded_amount`.
   All amount changes use validated positive amounts and checked arithmetic;
   funding cannot exceed the sum of milestone amounts.
5. **Authorization is short-lived where it should be.** A release approval is
   keyed by both contract ID and milestone index, must be live, and is cleared
   after settlement. Missing and expired approvals are equivalent.
6. **Settlement configuration is immutable.** `SettlementToken` is
   write-once after initialization. It cannot be the escrow contract or the
   admin address, and it is checked as a SAC before storage.
7. **Finalization is immutable.** Once `Finalization(id)` exists, protected
   per-contract mutations fail before changing state.
8. **TTL is part of availability.** Contract and milestone reads/writes renew
   their persistent entries together. Integrators needing long-lived escrows
   should keep both entries active; an evicted persistent record is not
   recoverable by a normal getter.

## Entrypoints that touch storage

| Entrypoint group | Keys read or written |
| --- | --- |
| Setup: `initialize`, `bind_settlement_token` | `Initialized`, `Admin`, `NextContractId`, `ReadinessChecklist`, `SettlementToken` |
| Creation: `create_contract` | `Initialized`, `Paused`, `Emergency`, `GovernedParameters`, `NextContractId`, `Contract(id)`, milestone vector |
| Funding and settlement: `deposit_funds`, `release_milestone`, `refund_unreleased_milestones`, `cancel_contract` | `SettlementToken`, `ProtocolFeeBps`, `AccumulatedProtocolFees`, `Contract(id)`, milestone vector, and release approvals where applicable |
| Release consent: `approve_milestone_release`, approval checks | `Contract(id)`, milestone vector, temporary `MilestoneApprovals(id, index)` |
| Governance and safety: fee, governed-parameter, admin-transfer, pause, and emergency entrypoints | `Admin`, `PendingAdmin`, `ProtocolFeeBps`, `GovernedParameters`, `ReadinessChecklist`, `Paused`, `Emergency` |
| Reputation and evidence: `submit_work_evidence`, `issue_reputation` | `Contract(id)`, milestone vector, `ReputationIssued(id)`, `PendingReputationCredits(address)`, `Reputation(address)`, `ReputationComment(id)` |
| Close and reads: `finalize_contract`, summaries, getters | `Finalization(id)`, `Contract(id)`, milestone vector, plus the relevant configuration and safety keys |
| Migration helpers | `Contract(id)` and temporary `PendingClientMigration(id)` |

## Worked example: create, fund, approve, and release milestone 0

Assume a newly initialized escrow, `NextContractId = 1`, and one milestone of
100 stroops.

1. `create_contract` validates the participants and milestone amount, writes
   `Contract(1)` with zero cumulative amounts, writes
   `(Contract(1), "milestones")` with one unreleased/unrefunded milestone, and
   advances `NextContractId` to 2.
2. `deposit_funds(1, client, 100)` transfers the bound SAC amount and writes
   the paired contract records so both `Contract.funded_amount` and the
   milestone's `funded_amount` are 100. The pair's persistent TTL is renewed.
3. `approve_milestone_release(1, 0, approver)` writes temporary
   `MilestoneApprovals(1, 0)` and gives it the seven-day approval TTL. The
   exact flags required depend on `Contract.release_authorization`.
4. `release_milestone(1, 0, caller)` verifies the live approval and the
   milestone vector, transfers the payout, marks `milestones[0].released`,
   increases `Contract.released_amount`, records any protocol fee, clears the
   temporary approval key, and renews the persistent pair.

At the end, the milestone vector and `Contract(1)` agree that the full
100-stroop obligation has been released; the temporary approval no longer
authorizes anything.

## Review and test pointers

The storage-focused coverage is in
[`contracts/escrow/src/test/storage.rs`](../contracts/escrow/src/test/storage.rs),
[`persistence.rs`](../contracts/escrow/src/test/persistence.rs), and
[`ttl_tests.rs`](../contracts/escrow/src/test/ttl_tests.rs). The allocation,
approval, migration, and finalization modules contain their own targeted
tests. Run `cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` from the
repository root before merging storage-related changes.
