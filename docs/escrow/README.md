# Escrow Integration Guide

This guide documents the entrypoints currently implemented by the escrow
contract. Planned features are listed separately and linked to their tracking
issues so integrators can distinguish live API from roadmap.

## Module Map

- `contracts/escrow/src/lib.rs`: contract type, shared API surface, reads, controls, cancellation, reputation, and module wiring.
- `contracts/escrow/src/create_contract.rs`: `create_contract` lifecycle entrypoint.
- `contracts/escrow/src/deposit.rs`: `deposit_funds` lifecycle entrypoint.
- `contracts/escrow/src/release.rs`: `release_milestone` and `release_milestones_batch` lifecycle entrypoints.
- `contracts/escrow/src/refund.rs`: `refund_unreleased_milestones` lifecycle entrypoint.

## Implemented API Surface

Lifecycle and reputation:

- `create_contract(client, freelancer, milestone_amounts, deposit_mode) -> u32`
- `deposit_funds(contract_id, amount) -> bool`
- `release_milestone(contract_id, caller, milestone_index) -> bool`
- `release_milestones_batch(contract_id, caller, milestone_indices: Vec<u32>) -> i128`
- `issue_reputation(contract_id, caller, freelancer, rating) -> bool`
- `cancel_contract(contract_id, caller) -> bool`
- `finalize_contract(contract_id, finalizer) -> bool`

Read-only queries:

- `get_contract(contract_id) -> EscrowContractData`
- `get_finalization_record(contract_id) -> Option<FinalizationRecord>`
- `get_reputation(freelancer) -> Option<ReputationRecord>`
- `get_average_rating(freelancer) -> Option<i128>`
- `get_pending_reputation_credits(freelancer) -> u32`
- `get_admin() -> Option<Address>`
- `is_paused() -> bool`
- `is_emergency() -> bool`
- `get_mainnet_readiness_info() -> MainnetReadinessInfo`

Operational controls:

- `initialize(admin) -> bool`
- `pause() -> bool`
- `unpause() -> bool`
- `activate_emergency_pause() -> bool`
- `resolve_emergency() -> bool`

## Canonical Happy Path

### 1. Initialize Operational Admin

```rust
escrow.initialize(&admin);
```

`initialize` is single-use, requires `admin.require_auth()`, and stores the
admin used by pause and emergency controls.

### 2. Create Contract

```rust
let contract_id = escrow.create_contract(
    &client_addr,
    &freelancer_addr,
    &vec![&env, 500_0000000_i128, 500_0000000_i128],
    &DepositMode::ExactTotal,
);
```

Creation requires `client.require_auth()`, rejects identical client/freelancer
addresses, rejects empty or non-positive milestones, caps milestone count at
`MAX_MILESTONES`, and caps total escrow value at `MAX_TOTAL_ESCROW_STROOPS`.

### 3. Deposit Funds

```rust
escrow.deposit_funds(&contract_id, &1000_0000000_i128);
```

`ExactTotal` contracts require one exact deposit equal to the milestone total.
`Incremental` contracts allow partial deposits until the milestone total is
reached. Deposits that exceed the required total fail closed.

### 4. Release Milestones

**Single release:**

```rust
escrow.release_milestone(&contract_id, &caller, &0);
```

**Batch release** (`release_milestones_batch` — resolves [#392](https://github.com/Talenttrust/Talenttrust-Contracts/issues/392)):

```rust
let total_released = escrow.release_milestones_batch(
    &contract_id,
    &caller,
    &vec![&env, 0_u32, 1_u32, 2_u32],
);
```

`release_milestones_batch` validates and releases a deduplicated set of
milestone indices atomically in a single transaction.

**All-or-nothing guarantee:** every index is fully validated (role check,
per-milestone approval, state, balance) before any state is mutated. If any
index fails, the entire call reverts with no partial side-effects.

**Errors specific to the batch entrypoint:**
- `EmptyBatchRelease` — index list was empty.
- `DuplicateMilestoneInBatch` — the same index appeared more than once.

All other errors (`UnauthorizedRole`, `IndexOutOfBounds`, `MilestoneAlreadyReleased`,
`AlreadyRefunded`, `InsufficientApprovals`, `InsufficientFunds`) carry the same
semantics as `release_milestone` but abort the entire batch.

**Events:** a single `("release", "batch")` event carrying `(contract_id,
total_amount)` is emitted on success. A `("protocol_fee", "batch")` event is
emitted when fees are configured.

When the last unsettled milestone is released, status becomes `Completed`.

### 5. Issue Reputation

```rust
escrow.issue_reputation(&contract_id, &client_addr, &freelancer_addr, &5_i128);
```

Reputation requires `caller.require_auth()`, the caller must be the stored
client, the freelancer argument must match the contract freelancer, the contract
must be `Completed`, rating must be `1..=5`, and each contract can issue
reputation once.

## Cancellation

```rust
escrow.cancel_contract(&contract_id, &caller);
```

Cancellation requires `caller.require_auth()`. The caller must be the stored
client or freelancer. It is blocked after `Completed` and blocked if the
contract is already `Cancelled`.

## Finalization

```rust
escrow.finalize_contract(&contract_id, &finalizer);
```

Finalization requires `finalizer.require_auth()`. The finalizer must be the
stored client, freelancer, or assigned arbiter. It is allowed only while the
contract status is `Completed` or `Disputed`.

The contract writes one immutable `FinalizationRecord` containing the finalizer,
ledger timestamp, and a `ContractSummary` snapshot. After the record exists,
contract-specific mutating calls reject with `AlreadyFinalized`.

## Pause and Emergency Controls

`pause`, `unpause`, `activate_emergency_pause`, and `resolve_emergency` require
the stored admin's authorization. While paused or in emergency, mutating
lifecycle calls fail with `ContractPaused`; read-only queries remain available.
`unpause` fails while emergency mode is active.

## Events

Implemented events:

- `("init", "admin_set")` on `initialize`
- `("paused", timestamp)` on `pause`
- `("unpaused", timestamp)` on `unpause`
- `("emergency", "activated")` and `("emergency", "resolved")`
- `("audit", contract_id)` for lifecycle state transitions
- `("created", contract_id)` on contract creation
- `("released", contract_id, milestone_index)` on `release_milestone`
- `("release", "batch") -> (contract_id, total_amount)` on `release_milestones_batch`
- `("protocol_fee", "batch") -> (contract_id, total_fee)` on batch release when fees are configured
- `("rep_issd", contract_id)` on reputation issuance
- `("cancelled", contract_id)` on cancellation
- `("finalized", contract_id)` on finalization

There is no dedicated deposit event in the current implementation unless the
deposit changes contract status and therefore emits an audit event. Structured
deposit and fee events are planned in
[#336](https://github.com/Talenttrust/Talenttrust-Contracts/issues/336).

## Implemented Security Assumptions

- Creation and reputation issue require explicit address authentication.
- Pause and emergency controls are admin-authenticated.
- Deposits cannot exceed the exact milestone total.
- Releases fail on duplicate milestone release, invalid milestone id, missing
  contract, paused state, and insufficient funded balance.
- Arithmetic for escrow totals, deposits, and releases uses checked helpers and
  panics with `PotentialOverflow` on overflow.
- Accounting is checked after balance-changing operations.
- The contract stores accounting state only; token custody and token transfers
  are not implemented in `lib.rs` and must be handled by an audited integration.
- Storage uses persistent keys. TTL constants exist for planned pending approval
  and migration flows, but no current public entrypoint writes those pending
  records.

## Planned Features

These features are not implemented entrypoints today:

- Two-step admin transfer: planned in
  [#318](https://github.com/Talenttrust/Talenttrust-Contracts/issues/318).
- Protocol fee deduction on release: planned in
  [#313](https://github.com/Talenttrust/Talenttrust-Contracts/issues/313).
- Protocol fee treasury withdrawal: planned in
  [#314](https://github.com/Talenttrust/Talenttrust-Contracts/issues/314).
- Governed parameter setter/readiness wiring: planned in
  [#323](https://github.com/Talenttrust/Talenttrust-Contracts/issues/323).
- Structured deposit and fee events: planned in
  [#336](https://github.com/Talenttrust/Talenttrust-Contracts/issues/336).
- Storage-key reference for declared-but-unused keys, including pending client
  migration and protocol fee keys: planned in
  [#342](https://github.com/Talenttrust/Talenttrust-Contracts/issues/342).
- `migrate_state` / `StateV1` / `StateV2` migration flow: not implemented;
  tracked by this reconciliation issue
  [#341](https://github.com/Talenttrust/Talenttrust-Contracts/issues/341)
  until a dedicated implementation issue exists.

Any documentation that describes one of these items as available should be
treated as roadmap text, not live integration guidance.
