# Milestone Deadlines and Timeout-Based Auto-Refund

## Overview

Milestones can optionally carry a **deadline** — a Unix timestamp (seconds) set at contract creation. After the deadline passes, the client may call `claim_timeout_refund` to recover the milestone funds without requiring the freelancer's cooperation or an arbiter's ruling.

## Deadline Mechanics

### Setting Deadlines

Deadlines are passed as an optional `Vec<u64>` parameter to `create_contract`. Each element corresponds to the milestone at the same index. If `Some(deadlines)` is provided, the vector length **must equal** the number of milestones.

```rust
pub fn create_contract(
    env: Env,
    client: Address,
    freelancer: Address,
    arbiter: Option<Address>,
    milestones: Vec<i128>,
    release_authorization: ReleaseAuthorization,
    deadlines: Option<Vec<u64>>,     // <-- new parameter
) -> u32;
```

- `None` — no milestone has a deadline (existing behavior preserved).
- `Some(vec![...])` — each entry is the deadline in Unix epoch seconds.

### Storage

The `deadline` field is stored inside the `Milestone` struct:

```rust
pub struct Milestone {
    pub amount: i128,
    pub funded_amount: i128,
    pub released: bool,
    pub refunded: bool,
    pub work_evidence: Option<String>,
    pub refunded_amount: i128,
    pub deadline: Option<u64>,       // <-- new field
}
```

The field is `Option<u64>`. A `None` deadline means the milestone never expires and cannot be timeout-refunded.

## `claim_timeout_refund` Entrypoint

```rust
pub fn claim_timeout_refund(
    env: Env,
    contract_id: u32,
    milestone_index: u32,
) -> i128
```

### Preconditions

| Check | Error |
|---|---|
| Pause / emergency gate | `ContractPaused` / `EmergencyActive` |
| Contract exists | `ContractNotFound` |
| Not finalized | `AlreadyFinalized` |
| Status is `Created`, `Funded`, or `PartiallyFunded` | `InvalidState` |
| Milestone index in bounds | `IndexOutOfBounds` |
| Milestone not released | `AlreadyReleased` |
| Milestone not refunded | `AlreadyRefunded` |
| Milestone has a deadline | `DeadlineNotPassed` |
| `now_seconds(env) > deadline` | `DeadlineNotPassed` |
| Caller is the stored client | auth failure (panic) |

### Accounting

- Milestone is marked `refunded = true`.
- `contract.refunded_amount` is incremented by the milestone amount.
- If all milestones become released-or-refunded:
  - If all are refunded → status = `Refunded`.
  - If some are released → status = `Completed`.
- The available-balance invariant is implicitly satisfied because the milestone was funded and not yet released or refunded.

### Event

```rust
("timeout", "refund")  →  (contract_id, milestone_index, amount)
```

## Security Considerations

1. **No refund before deadline**: `now_seconds <= deadline` causes `DeadlineNotPassed`. At-the-deadline is treated as "not passed".
2. **No double refund**: Already-refunded milestones are rejected with `AlreadyRefunded`.
3. **No refund on released milestones**: Rejected with `AlreadyReleased`.
4. **Client-only**: Only the contract client may call `claim_timeout_refund`. The freelancer cannot trigger a timeout refund against themself.
5. **Pause-respecting**: The pause/emergency gate runs before any state read.

## Usage Example

```rust
// Create a contract with a 7-day deadline on milestone 0
let deadlines = vec![&env, 7 * 86400_u64];
let id = client.create_contract(
    &client_addr,
    &freelancer_addr,
    &None,
    &vec![&env, 1000_i128],
    &ReleaseAuthorization::ClientOnly,
    &Some(deadlines),
);

// Advance ledger past deadline
env.ledger().set(LedgerInfo { timestamp: 7 * 86400 + 1, ..Default::default() });

// Client reclaims the milestone
let refunded = client.claim_timeout_refund(&id, &0_u32);
```

## Testing

Comprehensive tests live in `contracts/escrow/src/test/timeout_tests.rs`:

- Claim fails before deadline.
- Claim fails at exact deadline (not passed).
- Claim succeeds after deadline.
- Contract auto-completes when all milestones are released-or-refunded.
- No-deadline milestone cannot be timeout-refunded.
- Already-released or already-refunded milestones are rejected.
- Out-of-bounds index is rejected.
- Paused contract blocks the call.
- Invalid terminal state blocks the call.
- Event emission is verified.
- Multiple milestones with staggered deadlines work independently.