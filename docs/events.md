# Events Model Documentation

This document describes the events data model, its invariants, and the entrypoints that emit events in the TalentTrust escrow contract.

## Overview

The TalentTrust escrow contract uses Soroban's event system (`env.events().publish()`) to emit structured events for off-chain indexing, monitoring, and audit trails. Events are emitted after successful state mutations, ensuring that indexers observe only committed state transitions.

## Event Model Structure

All events follow the Soroban event model:
- **Topics**: An array of `Symbol` values that identify the event type and optionally include contextual identifiers (e.g., contract IDs)
- **Data**: The event payload containing relevant state information

### Event Naming Conventions

- **Short symbols**: Use `symbol_short!()` for frequently used, space-efficient identifiers (e.g., `"created"`, `"mlstn_rls"`)
- **Full symbols**: Use `Symbol::new()` for less common or longer identifiers (e.g., `"settlement_token_bound"`)
- **Hierarchical topics**: Some events use nested symbols for categorization (e.g., `("dispute", "opened")`)

## Event Catalog

### 1. Contract Lifecycle Events

#### 1.1 Contract Created

**Entrypoint**: `create_contract` (`create_contract.rs:168`)

**Topics**: `(symbol_short!("created"), contract_id: u32)`

**Data**: `(client: Address, freelancer: Address, timestamp: u64)`

**Description**: Emitted when a new escrow contract is successfully created. Contains the client and freelancer addresses along with the creation timestamp.

**Invariants**:
- Emitted only after all validation checks pass (distinct participants, valid milestones, arbiter requirements)
- Emitted only after the contract record is persisted to storage
- Emitted only after the milestone vector is persisted
- Emitted only after `NextContractId` is incremented
- The `contract_id` in the topic matches the ID returned to the caller
- The timestamp is the ledger timestamp at creation time

**Example**:
```rust
env.events().publish(
    (symbol_short!("created"), 42),
    (client_address, freelancer_address, 1704067200),
);
```

---

#### 1.2 Contract Finalized

**Entrypoint**: `finalize_contract` (`finalize.rs:162`)

**Topics**: `(symbol_short!("finalized"), contract_id: u32)`

**Data**: `(finalizer: Address, timestamp: u64)`

**Description**: Emitted when an escrow contract is finalized, writing immutable close metadata. Only allowed when the contract is in `Completed` or `Disputed` status.

**Invariants**:
- Emitted only after the contract is verified to be in `Completed` or `Disputed` status
- Emitted only after the finalizer is verified to be an authorized participant (client, freelancer, or arbiter)
- Emitted only after the `FinalizationRecord` is persisted to storage
- Emitted only after the contract is verified to not already be finalized
- The `finalizer` is the address that authorized the finalization call
- After this event, all contract-specific mutating entrypoints fail with `AlreadyFinalized`

**Example**:
```rust
env.events().publish(
    (symbol_short!("finalized"), 42),
    (finalizer_address, 1704153600),
);
```

---

#### 1.3 Contract Cancelled

**Entrypoint**: `cancel_contract` (`lib.rs:1645`)

**Topics**: `(symbol_short!("cancelled"), contract_id: u32)`

**Data**: `(client: Address, refund_amount: i128, timestamp: u64)`

**Description**: Emitted when a contract is cancelled before any milestone has been released. The full refundable balance is returned to the client.

**Invariants**:
- Emitted only after the contract is verified to be in `Created` or `Funded` status
- Emitted only after verification that no milestones have been released (`released_amount == 0`)
- Emitted only after the token transfer to the client succeeds (if `refund_amount > 0`)
- Emitted only after the contract status is set to `Cancelled`
- Emitted only after the contract record is persisted
- The `refund_amount` equals `funded_amount - released_amount - refunded_amount`
- If `refund_amount == 0`, no token transfer occurs (zero-funded cancellation)

**Example**:
```rust
env.events().publish(
    (symbol_short!("cancelled"), 42),
    (client_address, 1_000_000_00, 1704240000),
);
```

---

#### 1.4 Contract Completed

**Entrypoint**: `release_milestone` (`lib.rs:924`)

**Topics**: `(symbol_short!("ctrct_cmp"), contract_id: u32)`

**Data**: `(caller: Address, timestamp: u64)`

**Description**: Emitted when the final milestone is released, completing the contract. This event is emitted in addition to the `mlstn_rls` event for the final milestone.

**Invariants**:
- Emitted only when all milestones are either released or refunded
- Emitted only after the contract status is set to `Completed`
- Emitted only after a pending reputation credit is granted to the freelancer
- Emitted only after the contract and milestone storage is updated
- Always emitted alongside the final `mlstn_rls` event
- The `caller` is the address that authorized the final milestone release

**Example**:
```rust
env.events().publish(
    (symbol_short!("ctrct_cmp"), 42),
    (caller_address, 1704326400),
);
```

---

### 2. Milestone Events

#### 2.1 Milestone Released

**Entrypoint**: `release_milestone` (`lib.rs:907`)

**Topics**: `(symbol_short!("mlstn_rls"), contract_id: u32)`

**Data**: `(milestone_index: u32, amount: i128, fee: i128, new_released_amount: i128, caller: Address, timestamp: u64)`

**Description**: Emitted on every successful milestone release. Contains the milestone details, protocol fee, and updated accounting state.

**Invariants**:
- Emitted only after the milestone is verified to not already be released or refunded
- Emitted only after required approvals are verified (based on `ReleaseAuthorization` mode)
- Emitted only after sufficient funds are verified (`available_balance >= milestone.amount`)
- Emitted only after the protocol fee is calculated and accrued (if `fee_bps > 0`)
- Emitted only after the milestone `released` flag is set to `true`
- Emitted only after the contract `released_amount` is incremented
- Emitted only after approvals are cleared from temporary storage
- Emitted only after the contract and milestone storage is updated
- If this release completes the contract, a `ctrct_cmp` event is also emitted
- The `amount` is the gross milestone amount before fees
- The `fee` is the protocol fee calculated as `amount * fee_bps / 10_000`
- The `new_released_amount` is the cumulative released amount after this release

**Example**:
```rust
env.events().publish(
    (symbol_short!("mlstn_rls"), 42),
    (0, 1_000_000_00, 25_000, 1_000_000_00, caller_address, 1704412800),
);
```

---

#### 2.2 Milestone Released (Alternative)

**Entrypoint**: `release_milestone_impl` (`release.rs:140`)

**Topics**: `(Symbol::new(&env, "milestone_released"), contract_id)`

**Data**: `(caller: Address, milestone_index: u32, amount: i128)`

**Description**: Alternative milestone release event emitted from the implementation function. This is a simpler version of the milestone release event.

**Invariants**:
- Emitted only after the milestone is successfully released
- Emitted only after contract and milestone TTL is extended
- The `amount` matches the milestone amount
- The `milestone_index` is the index of the released milestone

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "milestone_released"), 42),
    (caller_address, 0, 1_000_000_00),
);
```

---

#### 2.3 Work Evidence Submitted

**Entrypoint**: `submit_work_evidence` (`lib.rs:1918`)

**Topics**: `(symbol_short!("evidence"), contract_id)`

**Data**: `(milestone_index: u32, freelancer: Address, timestamp: u64)`

**Description**: Emitted when a freelancer submits work evidence (e.g., IPFS CID or URL hash) for an unreleased milestone.

**Invariants**:
- Emitted only after the caller is verified to be the freelancer
- Emitted only after the contract is verified to be in `Funded` status
- Emitted only after the milestone is verified to not be released or refunded
- Emitted only after the evidence string is verified to be ≤ 256 bytes
- Emitted only after the evidence is persisted to the milestone
- Emitted only after the milestone storage is updated
- Emitted only after the contract TTL is extended
- The `freelancer` is the address that submitted the evidence
- Evidence may be overwritten before release (subsequent submissions replace previous ones)

**Example**:
```rust
env.events().publish(
    (symbol_short!("evidence"), 42),
    (0, freelancer_address, 1704500000),
);
```

---

### 3. Refund Events

#### 3.1 Milestones Refunded

**Entrypoint**: `refund_unreleased_milestones` (`lib.rs:1153`)

**Topics**: `(symbol_short!("refunded"), contract_id: u32)`

**Data**: `(total_refund_amount: i128, new_status: ContractStatus, timestamp: u64)`

**Description**: Emitted when unreleased milestones are refunded back to the client. The contract status may transition to `Refunded` or `Completed` depending on whether all milestones were refunded.

**Invariants**:
- Emitted only after all requested milestones are verified to not be released or refunded
- Emitted only after timeout conditions are verified (if deadlines are set)
- Emitted only after sufficient funds are verified
- Emitted only after the token transfer to the client succeeds
- Emitted only after all requested milestones are marked as refunded
- Emitted only after the contract `refunded_amount` is incremented
- Emitted only after the contract status is updated
- Emitted only after the contract and milestone storage is updated
- The `total_refund_amount` is the sum of all refunded milestone amounts
- The `new_status` is `Refunded` if all milestones were refunded, `Completed` if some were released
- If the new status is `Completed`, a pending reputation credit is granted to the freelancer

**Example**:
```rust
env.events().publish(
    (symbol_short!("refunded"), 42),
    (1_000_000_00, ContractStatus::Refunded, 1704586400),
);
```

---

### 4. Governance Events

#### 4.1 Protocol Fee Changed

**Entrypoint**: `set_protocol_fee_bps` (`governance.rs:50`)

**Topics**: `(Symbol::new(&env, "protocol_fee_bps"),)`

**Data**: `(old_bps: u32, new_bps: u32, admin: Address, timestamp: u64)`

**Description**: Emitted when the admin changes the protocol fee in basis points. The fee takes effect immediately for the next milestone release.

#### 4.2 Admin Proposed

**Entrypoint**: `propose_governance_admin_impl` (`governance.rs:93`)

**Topics**: `(symbol_short!("admin"), Symbol::new(env, "proposed"))`

**Data**: `(admin: Address, proposed: Address, timestamp: u64)`

**Description**: Emitted when the current admin proposes a new governance admin. The proposal is stored with a timelock.

**Invariants**:
- Emitted only after the current admin authorizes the proposal
- Emitted only after the proposal is persisted to storage with the proposal ledger sequence
- The `admin` is the current admin address
- The `proposed` is the new admin address
- The proposal cannot be accepted until the timelock elapses

**Example**:
```rust
env.events().publish(
    (symbol_short!("admin"), Symbol::new(env, "proposed")),
    (current_admin, proposed_admin, 1704672800),
);
```

---

#### 4.3 Admin Accepted

**Entrypoint**: `accept_governance_admin_impl` (`governance.rs:135`)

**Topics**: `(symbol_short!("admin"), Symbol::new(env, "accepted"))`

**Data**: `(old_admin: Address, new_admin: Address, timestamp: u64)`

**Description**: Emitted when a pending admin proposal is accepted and the admin rotation completes. The timelock must have elapsed.

**Invariants**:
- Emitted only after the proposed admin authorizes acceptance
- Emitted only after the timelock is verified to have elapsed
- Emitted only after the admin address is updated in storage
- Emitted only after the pending proposal is removed from storage
- The `old_admin` is the previous admin address
- The `new_admin` is the new admin address (previously the proposed address)

**Example**:
```rust
env.events().publish(
    (symbol_short!("admin"), Symbol::new(env, "accepted")),
    (old_admin, new_admin, 1704759200),
);
```

---

#### 4.4 Admin Proposal Cancelled

**Entrypoint**: `cancel_governance_admin_proposal_impl` (`governance.rs:174`)

**Topics**: `(symbol_short!("admin"), Symbol::new(env, "cancelled"))`

**Data**: `(admin: Address, cancelled_proposal: Address, timestamp: u64)`

**Description**: Emitted when the current admin cancels a pending governance admin proposal.

**Invariants**:
- Emitted only after the current admin authorizes cancellation
- Emitted only after a pending proposal is verified to exist
- Emitted only after the pending proposal is removed from storage
- The `admin` is the current admin address
- The `cancelled_proposal` is the address that was proposed

**Example**:
```rust
env.events().publish(
    (symbol_short!("admin"), Symbol::new(env, "cancelled")),
    (current_admin, proposed_admin, 1704845600),
);
```

---

### 5. Settlement Token Events

#### 5.1 Settlement Token Bound

**Entrypoint**: `bind_settlement_token` (`lib.rs:308`)

**Topics**: `(Symbol::new(&env, "settlement_token_bound"),)`

**Data**: `(admin: Address, token: Address, timestamp: u64)`

**Description**: Emitted when the admin binds a Stellar Asset Contract (SAC) token as the settlement token for custody transfers. This is a write-once operation.

**Invariants**:
- Emitted only after the admin authorizes the bind
- Emitted only after the pre-bind probe verifies the token implements the SAC interface
- Emitted only after verification that the token is not the escrow contract itself
- Emitted only after verification that the token is not the admin address
- Emitted only after verification that no token is already bound
- Emitted only after the token address is persisted to storage
- The event is not emitted if any validation fails (the entrypoint panics)
- After this event, all money-flow entrypoints use this token for SAC transfers

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "settlement_token_bound"),),
    (admin_address, token_address, 1704932000),
);
```

---

### 6. Initialization Events

#### 6.1 Contract Initialized

**Entrypoint**: `initialize` (`lib.rs:393`)

**Topics**: `(symbol_short!("init"), Symbol::new(&env, "admin_set"))`

**Data**: `(admin: Address, timestamp: u64)`

**Description**: Emitted when the escrow contract is initialized with the operational admin. This is a single-use operation.

**Invariants**:
- Emitted only after the admin authorizes initialization
- Emitted only after verification that the contract is not already initialized
- Emitted only after the admin address is persisted to storage
- Emitted only after the `Initialized` flag is set to `true`
- Emitted only after `NextContractId` is initialized to `1`
- Emitted only after the `ReadinessChecklist.initialized` flag is set to `true`
- After this event, all money-flow operations require initialization to succeed

**Example**:
```rust
env.events().publish(
    (symbol_short!("init"), Symbol::new(&env, "admin_set")),
    (admin_address, 1705018400),
);
```

---

### 7. Pause and Emergency Events

#### 7.1 Contract Paused

**Entrypoint**: `pause` (`lib.rs:1434`)

**Topics**: `(symbol_short!("pause"), timestamp: u64)`

**Data**: `(admin: Address,)`

**Description**: Emitted when the admin pauses all state-changing escrow operations. While paused, mutating entrypoints panic with `ContractPaused`.

**Invariants**:
- Emitted only after the admin authorizes the pause
- Emitted only after the `Paused` flag is set to `true` in storage
- Read-only queries are never blocked by pause
- The `admin` is the stored admin address

**Example**:
```rust
env.events().publish(
    (symbol_short!("pause"), 1705104800),
    (admin_address,),
);
```

---

#### 7.2 Contract Unpaused

**Entrypoint**: `unpause` (`lib.rs:1460`)

**Topics**: `(symbol_short!("unpaused"), timestamp: u64)`

**Data**: `(admin: Address,)`

**Description**: Emitted when the admin unpauses operations, clearing the `Paused` flag. Blocked while `Emergency` is active.

**Invariants**:
- Emitted only after the admin authorizes the unpause
- Emitted only after verification that `Emergency` is not active
- Emitted only after the `Paused` flag is set to `false` in storage
- The `admin` is the stored admin address

**Example**:
```rust
env.events().publish(
    (symbol_short!("unpaused"), 1705191200),
    (admin_address,),
);
```

---

#### 7.3 Emergency Activated

**Entrypoint**: `activate_emergency_pause` (`lib.rs:1514`)

**Topics**: `(Symbol::new(&env, "emergency"), Symbol::new(&env, "activated"))`

**Data**: `(admin: Address, timestamp: u64)`

**Description**: Emitted when the admin activates emergency pause, setting both `Emergency` and `Paused` flags. While emergency is active, all mutating entrypoints panic with `EmergencyActive` or `ContractPaused`, and `unpause` is blocked.

**Invariants**:
- Emitted only after the admin authorizes activation
- Emitted only after both `Emergency` and `Paused` flags are set to `true`
- Emitted only after the `ReadinessChecklist.emergency_controls_enabled` flag is set to `true`
- The `admin` is the stored admin address
- After this event, `unpause` is blocked until `resolve_emergency` is called

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "emergency"), Symbol::new(&env, "activated")),
    (admin_address, 1705277600),
);
```

---

#### 7.4 Emergency Resolved

**Entrypoint**: `resolve_emergency` (`lib.rs:1558`)

**Topics**: `(Symbol::new(&env, "emergency"), Symbol::new(&env, "resolved"))`

**Data**: `(admin: Address, timestamp: u64)`

**Description**: Emitted when the admin resolves emergency, clearing both `Emergency` and `Paused` flags. After resolution, all operations resume normally.

**Invariants**:
- Emitted only after the admin authorizes resolution
- Emitted only after both `Emergency` and `Paused` flags are set to `false`
- Emitted only after the `ReadinessChecklist.emergency_controls_enabled` flag is set to `true`
- The `admin` is the stored admin address
- After this event, normal operations resume

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "emergency"), Symbol::new(&env, "resolved")),
    (admin_address, 1705364000),
);
```

---

### 8. Dispute Events

#### 8.1 Dispute Opened

**Entrypoint**: `raise_dispute` (`lib.rs:2223`)

**Topics**: `(symbol_short!("dispute"), symbol_short!("opened"))`

**Data**: `(contract_id: u32, caller: Address)`

**Description**: Emitted when a client or freelancer opens a dispute for a funded or partially funded escrow contract. The contract status transitions to `Disputed`.

**Invariants**:
- Emitted only after the caller is verified to be the client or freelancer
- Emitted only after an arbiter is verified to be assigned to the contract
- Emitted only after the contract is verified to be in `Funded` or `PartiallyFunded` status
- Emitted only after the contract status is set to `Disputed`
- Emitted only after the contract record is persisted
- Emitted only after the contract TTL is extended
- The `caller` is the address that opened the dispute
- After this event, milestone releases are blocked until the dispute is resolved

**Example**:
```rust
env.events().publish(
    (symbol_short!("dispute"), symbol_short!("opened")),
    (42, caller_address),
);
```

---

#### 8.2 Dispute Resolved

**Entrypoint**: `resolve_dispute` (`lib.rs:2316`)

**Topics**: `(symbol_short!("dispute"), symbol_short!("resolved"))`

**Data**: `(contract_id: u32, resolution_code: u32)`

**Description**: Emitted when an arbiter resolves an open dispute by applying a resolution (FullRefund, PartialRefund, FullPayout, or Split). The resolution code indicates the type of resolution applied.

**Invariants**:
- Emitted only after the caller is verified to be the assigned arbiter
- Emitted only after the contract is verified to be in `Disputed` status
- Emitted only after the resolution payouts are calculated and verified
- Emitted only after the contract accounting is updated (`refunded_amount`, `released_amount`)
- Emitted only after the contract status is set based on the resolution outcome
- Emitted only after the contract record is persisted
- Emitted only after the contract TTL is extended
- The `resolution_code` corresponds to the `DisputeResolution` variant:
  - `0` = FullRefund
  - `1` = PartialRefund
  - `2` = FullPayout
  - `3` = Split
- If the final status is `Completed`, a pending reputation credit is granted to the freelancer

**Example**:
```rust
env.events().publish(
    (symbol_short!("dispute"), symbol_short!("resolved")),
    (42, 0), // 0 = FullRefund
);
```

---

### 9. Client Migration Events

#### 9.1 Client Migration Proposed

**Entrypoint**: `propose_client_migration_impl` (`migration.rs:85`)

**Topics**: `(Symbol::new(&env, "client_migration_proposed"), contract_id)`

**Data**: `(current_client: Address, new_client: Address, requested_at: u32)`

**Description**: Emitted when the current client proposes migrating the contract to a new client address. The proposal is stored in temporary storage with a TTL.

**Invariants**:
- Emitted only after the current client authorizes the proposal
- Emitted only after the new client is verified to not be the freelancer or current client
- Emitted only after the contract status is verified to allow migration
- Emitted only after verification that no pending migration already exists
- Emitted only after the proposal is persisted with TTL
- The `requested_at` is the ledger sequence when the proposal was created
- The proposal expires after `PENDING_MIGRATION_TTL_LEDGERS`

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "client_migration_proposed"), 42),
    (current_client, new_client, 12345),
);
```

---

#### 9.2 Client Migration Accepted

**Entrypoint**: `accept_client_migration_impl` (`migration.rs:120`)

**Topics**: `(Symbol::new(&env, "client_migration_accepted"), contract_id)`

**Data**: `(current_client: Address, new_client: Address, timestamp: u64)`

**Description**: Emitted when a proposed client migration is accepted and the contract's client address is updated.

**Invariants**:
- Emitted only after the proposed client authorizes acceptance
- Emitted only after the pending proposal is verified to exist and be live
- Emitted only after the proposed client matches the proposal
- Emitted only after the current client in the contract matches the proposal
- Emitted only after the contract's client address is updated
- The `timestamp` is the ledger timestamp at acceptance time

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "client_migration_accepted"), 42),
    (current_client, new_client, 1705450400),
);
```

---

#### 9.3 Client Migration Cancelled

**Entrypoint**: `cancel_client_migration` (`migration.rs:150`)

**Topics**: `(Symbol::new(&env, "client_migration_cancelled"), contract_id)`

**Data**: `(current_client: Address, timestamp: u64)`

**Description**: Emitted when the current client cancels a pending client migration proposal.

**Invariants**:
- Emitted only after the current client authorizes cancellation
- Emitted only after the caller is verified to be the contract's client
- Emitted only after a live pending migration is verified to exist
- Emitted only after the pending migration entry is removed from storage
- The `current_client` is the address that cancelled the migration

**Example**:
```rust
env.events().publish(
    (Symbol::new(&env, "client_migration_cancelled"), 42),
    (current_client, 1705536800),
);
```

---

### 10. Protocol Fee Events

#### 10.1 Protocol Fee Withdrawn

**Entrypoint**: `withdraw_protocol_fees` (`lib.rs:2065`)

**Topics**: `(symbol_short!("fee"), symbol_short!("withdraw"))`

**Data**: `(admin: Address, to: Address, amount: i128, timestamp: u64)`

**Description**: Emitted when the admin withdraws accrued protocol fees from the escrow contract to a treasury address.

**Invariants**:
- Emitted only after the admin authorizes the withdrawal
- Emitted only after the amount is verified to be positive
- Emitted only after sufficient accumulated fees are verified
- Emitted only after the `AccumulatedProtocolFees` balance is decremented
- Emitted only after the token transfer to the treasury succeeds
- Emitted only after the fee storage TTL is extended
- The `amount` is the amount withdrawn (must be ≤ accumulated fees)
- The `to` is the treasury address receiving the fees

**Example**:
```rust
env.events().publish(
    (symbol_short!("fee"), symbol_short!("withdraw")),
    (admin_address, treasury_address, 50_000_00, 1705623200),
);
```

---

## Event Invariants Summary

### Universal Invariants

All events in the contract adhere to these universal invariants:

1. **Post-Mutation Emission**: Events are emitted only after all state mutations succeed. If an entrypoint panics during validation or state mutation, no event is emitted.

2. **Atomicity**: Events are emitted atomically with the state transition. Either both the state mutation and event emission succeed, or neither does.

3. **Timestamp Accuracy**: All events include a `timestamp` field that reflects the ledger timestamp at the moment of emission.

4. **Authorization**: All events from mutating entrypoints are emitted only after the caller's authorization is verified via `require_auth()`.

5. **Pause/Emergency Gates**: All mutating entrypoints check pause and emergency gates before state mutation. Events are not emitted if these gates block the operation.

6. **Initialization Gate**: All money-flow operations require initialization. Events are not emitted if the contract is not initialized.

### Category-Specific Invariants

#### Contract Lifecycle

- **Creation**: Contract ID is unique and monotonically increasing
- **Finalization**: Finalization is irreversible and blocks all future mutations
- **Cancellation**: Only possible before any milestone is released
- **Completion**: Only occurs when all milestones are released or refunded

#### Milestone Operations

- **Release**: Requires valid approvals based on `ReleaseAuthorization` mode
- **Evidence**: Can be overwritten before milestone release
- **Refund**: Requires milestones to be unreleased and (if deadlines exist) overdue

#### Governance

- **Admin Rotation**: Two-step process with timelock enforcement
- **Protocol Fees**: Changes take effect immediately for subsequent releases
- **Settlement Token**: Write-once operation with pre-bind validation

#### Disputes

- **Opening**: Requires arbiter assignment and disputable contract status
- **Resolution**: Only the assigned arbiter can resolve
- **Accounting**: Resolution must conserve available funds

#### Client Migration

- **Proposal**: Stored in temporary storage with TTL
- **Acceptance**: Only the proposed client can accept
- **Cancellation**: Only the current client can cancel

## Worked Example

### Complete Contract Lifecycle Event Flow

This example demonstrates the event sequence for a complete contract lifecycle from creation to completion.

#### Setup

```rust
// Addresses
let admin = Address::generate(&env);
let client = Address::generate(&env);
let freelancer = Address::generate(&env);
let token = Address::generate(&env);
```

#### 1. Initialize Contract

```rust
escrow.initialize(&admin);
```

**Event Emitted**:
```
Topics: ("init", "admin_set")
Data: (admin, 1704067200)
```

#### 2. Bind Settlement Token

 ```rust
escrow.bind_settlement_token(&admin, &token);
```

**Event Emitted**:
```
Topics: ("settlement_token_bound",)
Data: (admin, token, 1704067300)
```

#### 3. Create Contract

```rust
let milestones = vec![&env, 100_000_00, 150_000_00, 250_000_00];
let contract_id = escrow.create_contract(
    &client,
    &freelancer,
    None, // no arbiter
    milestones,
    ReleaseAuthorization::ClientOnly
);
```

**Event Emitted**:
```
Topics: ("created", 1)
Data: (client, freelancer, 1704067400)
```

#### 4. Set Protocol Fee

```rust
escrow.set_protocol_fee_bps(&250); // 2.5%
```

**Event Emitted**:
```
Topics: ("protocol_fee_bps",)
Data: (0, 250, admin, 1704067500)
```

#### 5. Deposit Funds

```rust
escrow.deposit_funds(&contract_id, &client, &500_000_00);
```

**No event emitted** (deposit is a token transfer, not a state-mutating operation that emits events)

#### 6. Submit Work Evidence

```rust
escrow.submit_work_evidence(&contract_id, &freelancer, &0, &"QmHash123...");
```

**Event Emitted**:
```
Topics: ("evidence", 1)
Data: (0, freelancer, 1704067600)
```

#### 7. Release First Milestone

```rust
escrow.approve_milestone_release(&contract_id, &client, &0);
escrow.release_milestone(&contract_id, &client, &0);
```

**Events Emitted**:
```
Topics: ("mlstn_rls", 1)
Data: (0, 100_000_00, 2_500, 100_000_00, client, 1704067700)
```

#### 8. Release Second Milestone

```rust
escrow.approve_milestone_release(&contract_id, &client, &1);
escrow.release_milestone(&contract_id, &client, &1);
```

**Events Emitted**:
```
Topics: ("mlstn_rls", 1)
Data: (1, 150_000_00, 3_750, 250_000_00, client, 1704067800)
```

#### 9. Release Final Milestone

```rust
escrow.approve_milestone_release(&contract_id, &client, &2);
escrow.release_milestone(&contract_id, &client, &2);
```

**Events Emitted**:
```
Topics: ("mlstn_rls", 1)
Data: (2, 250_000_00, 6_250, 500_000_00, client, 1704067900)

Topics: ("ctrct_cmp", 1)
Data: (client, 1704067900)
```

#### 10. Withdraw Protocol Fees

```rust
escrow.withdraw_protocol_fees(&12_500, &treasury);
```

**Event Emitted**:
```
Topics: ("fee", "withdraw")
Data: (admin, treasury, 12_500, 1704068000)
```

#### 11. Finalize Contract

```rust
escrow.finalize_contract(&contract_id, &client);
```

**Event Emitted**:
```
Topics: ("finalized", 1)
Data: (client, 1704068100)
```

### Event Summary

The complete lifecycle emitted the following events in order:

1. `("init", "admin_set")` - Contract initialization
2. `("settlement_token_bound",)` - Token binding
3. `("created", 1)` - Contract creation
4. `("protocol_fee_bps",)` - Protocol fee change
5. `("evidence", 1)` - Work evidence submission
6. `("mlstn_rls", 1)` - First milestone release
7. `("mlstn_rls", 1)` - Second milestone release
8. `("mlstn_rls", 1)` - Final milestone release
9. `("ctrct_cmp", 1)` - Contract completion
10. `("fee", "withdraw")` - Protocol fee withdrawal
11. `("finalized", 1)` - Contract finalization

## Entrypoint-Event Mapping

| Entrypoint | Module | Event(s) Emitted |
|------------|--------|------------------|
| `initialize` | `lib.rs` | `("init", "admin_set")` |
| `bind_settlement_token` | `lib.rs` | `("settlement_token_bound",)` |
| `create_contract` | `create_contract.rs` | `("created", contract_id)` |
| `set_protocol_fee_bps` | `governance.rs` | `("protocol_fee_bps",)` |
| `propose_governance_admin` | `governance.rs` | `("admin", "proposed")` |
| `accept_governance_admin` | `governance.rs` | `("admin", "accepted")` |
| `cancel_governance_admin_proposal` | `governance.rs` | `("admin", "cancelled")` |
| `deposit_funds` | `lib.rs` | None |
| `approve_milestone_release` | `lib.rs` | None |
| `release_milestone` | `lib.rs` | `("mlstn_rls", contract_id)`, `("ctrct_cmp", contract_id)`* |
| `refund_unreleased_milestones` | `lib.rs` | `("refunded", contract_id)` |
| `cancel_contract` | `lib.rs` | `("cancelled", contract_id)` |
| `submit_work_evidence` | `lib.rs` | `("evidence", contract_id)` |
| `issue_reputation` | `lib.rs` | None |
| `pause` | `lib.rs` | `("pause", timestamp)` |
| `unpause` | `lib.rs` | `("unpaused", timestamp)` |
| `activate_emergency_pause` | `lib.rs` | `("emergency", "activated")` |
| `resolve_emergency` | `lib.rs` | `("emergency", "resolved")` |
| `withdraw_protocol_fees` | `lib.rs` | `("fee", "withdraw")` |
| `raise_dispute` | `lib.rs` | `("dispute", "opened")` |
| `resolve_dispute` | `lib.rs` | `("dispute", "resolved")` |
| `propose_client_migration` | `migration.rs` | `("client_migration_proposed", contract_id)` |
| `accept_client_migration` | `migration.rs` | `("client_migration_accepted", contract_id)` |
| `cancel_client_migration` | `migration.rs` | `("client_migration_cancelled", contract_id)` |
| `finalize_contract` | `finalize.rs` | `("finalized", contract_id)` |

*Only emitted when the release completes the contract

## Testing Guidance

When testing events, verify the following:

1. **Event Presence**: Confirm that the expected event is emitted after successful operations
2. **Event Absence**: Confirm that no event is emitted when operations fail (panic)
3. **Topic Accuracy**: Verify that event topics match the expected symbols and identifiers
4. **Data Integrity**: Verify that event data fields contain correct values
5. **Ordering**: Verify that events are emitted in the correct sequence for multi-step operations
6. **Timestamp Accuracy**: Verify that timestamps reflect the ledger time
7. **Authorization**: Verify that events are only emitted after successful authorization

See `contracts/escrow/src/test/governance_events.rs` for examples of event testing patterns.
