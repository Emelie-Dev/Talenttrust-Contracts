# Requirements Document

## Introduction

This feature adds dedicated, indexer-friendly Soroban events for every meaningful state change in the TalentTrust escrow contract. Currently, only the milestone release step emits an event; contract creation, fund deposit, contract completion, and all dispute lifecycle transitions emit nothing. Off-chain indexers and analytics dashboards therefore have an incomplete view of contract state.

The feature covers two event families:

- **Milestone events** (`ms_state` topic) — emitted on contract creation, fund deposit, milestone release, and contract completion.
- **Dispute events** (`dsp_state` topic) — emitted when a dispute is opened and when it is resolved, along with the financial settlement amounts.

The dispute subsystem (data types, state enum, and contract functions) does not yet exist and must be designed and implemented as part of this work. No existing fund-transfer logic, authorization checks, or state transitions may be altered; events are strictly observational side effects.

---

## Glossary

- **Escrow**: The smart contract (`contracts/escrow/src/lib.rs`) that holds funds and coordinates payment between a client and a freelancer.
- **Contract_ID**: The `u32` identifier assigned to an escrow contract instance at creation time.
- **Milestone_ID**: A zero-based `u32` index into the milestones vector of a given escrow contract.
- **Milestone**: A `Milestone` struct with `amount: i128` and `released: bool` that records one payment tranche.
- **EscrowState**: The persistent storage struct holding `client`, `freelancer`, `milestones`, and `status` for one escrow contract.
- **ContractStatus**: The existing `enum` with variants `Created`, `Funded`, `Completed`, and `Disputed`.
- **Dispute**: A new storage struct representing a dispute record with a unique `Dispute_ID`, the associated `Contract_ID`, the current `DisputeStatus`, a `refunded_amount: i128`, and a `released_amount: i128`.
- **Dispute_ID**: A `u32` identifier assigned to a dispute record at the time the dispute is opened.
- **DisputeStatus**: A new `enum` with exactly the variants `Open` and `Resolved`, representing the lifecycle state of a dispute.
- **Event_System**: The Soroban `env.events().publish(topics, data)` mechanism used to emit contract events.
- **Indexer**: An off-chain service that subscribes to contract events to build a queryable history of state changes.
- **symbol_short!**: A Soroban macro that creates a `Symbol` from a string of at most 9 characters; used for event topics.
- **ms_state**: The 8-character topic `Symbol` used to identify milestone state-change events.
- **dsp_state**: The 9-character topic `Symbol` used to identify dispute state-change events.

---

## Requirements

### Requirement 1: Milestone Event on Contract Creation

**User Story:** As an indexer operator, I want to receive a `ms_state` event when a new escrow contract is created, so that I can record the initial state of every milestone from the first moment they exist on-chain.

#### Acceptance Criteria

1. WHEN `create_contract` stores a new `EscrowState` in persistent storage, THE Event_System SHALL publish one `ms_state` event per milestone in the newly created contract, in ascending `milestone_id` order (index 0 first, then 1, then 2, etc.).
2. THE Event_System SHALL publish each creation event with the topic tuple `(symbol_short!("ms_state"), contract_id, milestone_id)` where `milestone_id` is the zero-based index of the milestone.
3. THE Event_System SHALL publish each creation event with a `Milestone` payload where `amount` equals the value from the `milestone_amounts` input vector at index `milestone_id` and `released` equals `false`.
4. WHEN `create_contract` is called with an empty `milestone_amounts` vector, THE Event_System SHALL publish zero `ms_state` events.
5. IF `create_contract` is called with a `milestone_amounts` vector containing a non-positive amount (i.e., `amount <= 0`), THEN THE Escrow SHALL return an error and publish zero `ms_state` events.

---

### Requirement 2: Milestone Event on Fund Deposit

**User Story:** As an indexer operator, I want to receive a `ms_state` event when funds are deposited into an escrow contract, so that I can record that the contract has transitioned to the `Funded` state and all milestones are now backed by deposited funds.

#### Acceptance Criteria

1. WHEN `deposit_funds` transitions the `EscrowState` status to `ContractStatus::Funded`, THE Event_System SHALL publish one `ms_state` event per milestone in that contract, in ascending `milestone_id` order.
2. WHEN `deposit_funds` publishes deposit events, THE Event_System SHALL publish each event with the topic tuple `(symbol_short!("ms_state"), contract_id, milestone_id)` where `milestone_id` is the zero-based index of the milestone.
3. WHEN `deposit_funds` publishes deposit events, THE Event_System SHALL publish each event with a `Milestone` payload where `amount` equals the stored milestone amount (unchanged by the deposit) and `released` equals `false`.
4. IF `deposit_funds` is called on a `Contract_ID` that does not exist in persistent storage, THEN THE Escrow SHALL return an error without publishing any events.
5. IF `deposit_funds` is called on a contract whose `EscrowState` status is already `ContractStatus::Funded`, THEN THE Escrow SHALL return an error without publishing any events.

---

### Requirement 3: Milestone Event on Milestone Release

**User Story:** As an indexer operator, I want to receive a `ms_state` event when a specific milestone is released, so that I can record the exact payment tranche that was approved and the updated milestone state.

#### Acceptance Criteria

1. WHEN `release_milestone` transitions a milestone's `released` field from `false` to `true`, THE Event_System SHALL publish exactly one `ms_state` event with the topic tuple `(symbol_short!("ms_state"), contract_id, milestone_id)` and a `Milestone` payload where `amount` equals the milestone's stored amount and `released` equals `true`.
2. IF `release_milestone` is called and `require_auth()` rejects the caller, THEN THE Escrow SHALL abort without publishing any `ms_state` event.
3. IF `release_milestone` is called with a `contract_id` that does not exist in persistent storage, or a `milestone_id` that is out of bounds for that contract's milestones vector, THEN THE Escrow SHALL return an error without publishing any `ms_state` event.
4. WHILE a milestone's `released` field is already `true`, THE Event_System SHALL NOT publish a duplicate `ms_state` event when `release_milestone` is called for that milestone.

---

### Requirement 4: Milestone Event on Contract Completion

**User Story:** As an indexer operator, I want to receive a `ms_state` event when a contract reaches the `Completed` status, so that I can mark the entire engagement as finished in my off-chain index without polling.

#### Acceptance Criteria

1. WHEN `release_milestone` causes all milestones to be released, THE Event_System SHALL publish exactly one `ms_state` event for the final milestone — this single event serves as both the per-milestone release event (Requirement 3) and the contract-completion event. The implementation SHALL NOT publish a second separate event for the same milestone release.
2. THE Event_System SHALL publish the completion event with the topic tuple `(symbol_short!("ms_state"), contract_id, milestone_id)` where `milestone_id` is the argument passed to the `release_milestone` call that caused the all-released condition.
3. WHEN the completion event is published, THE Escrow SHALL have already written `ContractStatus::Completed` to persistent storage before the event is emitted.
4. IF releasing a milestone does not result in all milestones being released, THEN THE Event_System SHALL NOT publish any contract-completion `ms_state` event beyond the single per-milestone event already required by Requirement 3.
5. WHEN `release_milestone` is called and all milestones become released, the `ms_state` event payload SHALL contain only the `Milestone` struct fields (`amount` and `released`). An indexer SHALL infer contract completion by checking whether, after receiving a `ms_state` event with `released = true`, all milestones for that `contract_id` are now released — there is no additional completion-specific field in the payload.

---

### Requirement 5: Dispute Data Model

**User Story:** As a contract developer, I want a well-typed dispute data model, so that dispute records can be stored, retrieved, and emitted in events with a consistent structure.

#### Acceptance Criteria

1. THE Escrow SHALL define a `DisputeStatus` enum annotated with `#[contracttype]` containing exactly the variants `Open` and `Resolved`.
2. THE Escrow SHALL define a `Dispute` struct annotated with `#[contracttype]` containing the fields `contract_id: u32`, `status: DisputeStatus`, `refunded_amount: i128` (must be >= 0), and `released_amount: i128` (must be >= 0).
3. THE Escrow SHALL store `Dispute` records in persistent storage using a namespaced key (e.g., a `(Symbol, u32)` tuple such as `(symbol_short!("dispute"), dispute_id)`) to prevent key collisions with `EscrowState` records stored under bare `u32` keys.
4. THE Escrow SHALL assign `Dispute_ID` values starting at `1` and incrementing by `1` for each new dispute, using a persistent counter keyed separately from `Dispute` and `EscrowState` records.
5. THE Escrow SHALL initialize new `Dispute` records with `status` equal to `DisputeStatus::Open`, `refunded_amount` equal to `0`, and `released_amount` equal to `0`.
6. IF a caller attempts to retrieve a `Dispute` by a `Dispute_ID` that does not exist in persistent storage, THEN THE Escrow SHALL return an error.
7. THE Escrow SHALL use only types from `soroban_sdk` and Rust's `core` library when defining `Dispute` and `DisputeStatus`, maintaining `#![no_std]` compliance.

---

### Requirement 6: Dispute Event on Opening a Dispute

**User Story:** As an indexer operator, I want to receive a `dsp_state` event when a dispute is opened against an escrow contract, so that I can record the dispute and begin tracking its resolution in my off-chain index.

#### Acceptance Criteria

1. WHEN `open_dispute` is called on a contract whose `EscrowState` status is `ContractStatus::Funded`, THE Event_System SHALL publish exactly one `dsp_state` event.
2. THE Event_System SHALL publish the open-dispute event with the topic tuple `(symbol_short!("dsp_state"), dispute_id, contract_id)`.
3. THE Event_System SHALL publish the open-dispute event with a `Dispute` payload where `contract_id` equals the `contract_id` argument, `status` equals `DisputeStatus::Open`, `refunded_amount` equals `0`, and `released_amount` equals `0`.
4. WHEN `open_dispute` is called, THE Escrow SHALL transition the associated `EscrowState` status to `ContractStatus::Disputed` and persist the new `Dispute` record in storage before publishing the event.
5. IF `open_dispute` is called on a `Contract_ID` whose `EscrowState` status is already `ContractStatus::Disputed`, THEN THE Escrow SHALL return an error without modifying any state or publishing any events.
6. IF `open_dispute` is called on a `Contract_ID` that does not exist in persistent storage, THEN THE Escrow SHALL return an error without publishing any events.
7. WHEN `open_dispute` is called and `require_auth()` rejects the caller, THE Escrow SHALL abort without modifying any state or publishing any `dsp_state` event. The `open_dispute` function SHALL require authorization from the `client` address recorded in the `EscrowState`.

---

### Requirement 7: Dispute Event on Dispute Resolution

**User Story:** As an indexer operator, I want to receive a `dsp_state` event when a dispute is resolved, including the settlement amounts, so that I can record the final financial outcome in my off-chain index.

#### Acceptance Criteria

1. WHEN `resolve_dispute` is called on a dispute whose `DisputeStatus` is `Open`, THE Event_System SHALL publish exactly one `dsp_state` event.
2. WHEN `resolve_dispute` publishes the resolution event, THE Event_System SHALL use the topic tuple `(symbol_short!("dsp_state"), dispute_id, contract_id)`.
3. WHEN `resolve_dispute` publishes the resolution event, THE Event_System SHALL include a `Dispute` payload where `status` equals `DisputeStatus::Resolved`, `refunded_amount` equals the `refund_amount` argument (which must be >= 0), and `released_amount` equals the `release_amount` argument (which must be >= 0). The sum `refund_amount + release_amount` SHALL NOT exceed the total escrow amount for the associated contract.
4. WHEN `resolve_dispute` is called, THE Escrow SHALL update the stored `Dispute` record's `status` to `DisputeStatus::Resolved` and record the settlement amounts in persistent storage before publishing the event.
5. IF `resolve_dispute` is called on a `Dispute_ID` whose `DisputeStatus` is already `Resolved`, THEN THE Escrow SHALL abort (e.g., via `panic!`) without modifying any stored state or publishing any events.
6. WHEN `resolve_dispute` is called and `require_auth()` rejects the caller, THE Escrow SHALL abort without modifying any state or publishing any `dsp_state` event. The `resolve_dispute` function SHALL require authorization from the `client` address recorded in the associated `EscrowState`, and state SHALL remain unchanged on auth failure.
7. IF `resolve_dispute` is called with a `Dispute_ID` that does not exist in persistent storage, THEN THE Escrow SHALL return an error without publishing any events.

---

### Requirement 8: Event Topic Constraints and Uniqueness

**User Story:** As a contract developer, I want all event topics to satisfy `symbol_short!` length constraints and be globally unique within the contract, so that indexers can reliably distinguish event types without ambiguity.

#### Acceptance Criteria

1. THE Event_System SHALL use `symbol_short!("ms_state")` (8 characters) as topic[0] of every milestone state-change event. Topic[1] SHALL be `contract_id` (`u32`) and topic[2] SHALL be `milestone_id` (`u32`).
2. THE Event_System SHALL use `symbol_short!("dsp_state")` (9 characters) as topic[0] of every dispute state-change event. Topic[1] SHALL be `dispute_id` (`u32`) and topic[2] SHALL be `contract_id` (`u32`).
3. THE Escrow SHALL NOT publish any event whose topic[0] string exceeds 9 characters.
4. THE Escrow SHALL NOT use `symbol_short!("ms_state")` as topic[0] for any dispute event, and SHALL NOT use `symbol_short!("dsp_state")` as topic[0] for any milestone event.

---

### Requirement 9: No Alteration of Existing Business Logic

**User Story:** As a contract developer, I want the event additions to be purely observational, so that existing authorization checks, fund transfer logic, and state transitions are not modified or broken.

#### Acceptance Criteria

1. THE Escrow SHALL preserve all existing `require_auth()` call sites in `create_contract`, `deposit_funds`, and `release_milestone` without modification.
2. WHEN events are added to `create_contract`, `deposit_funds`, and `release_milestone`, THE Escrow SHALL call `env.events().publish(...)` only after `env.storage().persistent().set(...)` has been called for the primary state update in that function.
3. THE Escrow SHALL NOT include any token transfer logic in `open_dispute` or `resolve_dispute`; those functions record state and emit events only — they do not move funds.
4. THE Escrow SHALL NOT introduce any `use std::...` imports, and SHALL NOT add any new entries to `Cargo.toml` under `[dependencies]` or `[dev-dependencies]` that are not already present.

---

### Requirement 10: Test Coverage for All Events

**User Story:** As a developer, I want comprehensive tests for every event defined in this feature, so that regressions are caught immediately and the event schema is validated against the specification.

#### Acceptance Criteria

1. THE Test_Suite SHALL include a test that calls `create_contract` with three milestone amounts and asserts that `env.events().all()` has exactly 3 entries, each with topic[0] equal to `symbol_short!("ms_state")`, topic[1] equal to `escrow_id`, topic[2] equal to the milestone index, and a `Milestone` payload with the correct `amount` and `released = false`.
2. THE Test_Suite SHALL include a test that calls `deposit_funds` (after `create_contract`) and asserts that `env.events().all()` contains one `ms_state` event per milestone with topic[0] equal to `symbol_short!("ms_state")`, topic[1] equal to `escrow_id`, topic[2] equal to the milestone index, and a `Milestone` payload with the correct `amount` and `released = false`.
3. THE Test_Suite SHALL include a test that calls `release_milestone` for a single milestone and asserts that `env.events().all()` contains exactly one entry with topic[0] equal to `symbol_short!("ms_state")`, topic[1] equal to `escrow_id`, topic[2] equal to the released `milestone_id`, and a `Milestone` payload where `released` is `true`.
4. THE Test_Suite SHALL include a test that releases all milestones sequentially, and for the final `release_milestone` call asserts that exactly one `ms_state` event is emitted (not two), confirming the release and completion are the same single publish.
5. THE Test_Suite SHALL include a test that calls `open_dispute` and asserts that `env.events().all()` contains exactly one entry with topic[0] equal to `symbol_short!("dsp_state")`, topic[1] equal to `dispute_id`, topic[2] equal to `contract_id`, and a `Dispute` payload where `status` is `DisputeStatus::Open`, `refunded_amount` is `0`, and `released_amount` is `0`.
6. THE Test_Suite SHALL include a test that calls `resolve_dispute` (after `open_dispute`) and asserts that `env.events().all()` contains exactly one entry with topic[0] equal to `symbol_short!("dsp_state")`, topic[1] equal to `dispute_id`, topic[2] equal to `contract_id`, and a `Dispute` payload where `status` is `DisputeStatus::Resolved`, `refunded_amount` matches the argument, and `released_amount` matches the argument.
7. THE Test_Suite SHALL include an edge-case test that calls `open_dispute` on a contract already in `ContractStatus::Disputed` state, and asserts both that the call returns an error and that `env.events().all()` is empty (zero events published).
8. THE Test_Suite SHALL assert event structure using `env.events().all()` with explicit index-based access: `event.0.get(0)` for topic[0], `event.0.get(1)` for topic[1], `event.0.get(2)` for topic[2], and `event.1` for the typed data payload — consistent with the existing `test_release_milestone_event` pattern.
