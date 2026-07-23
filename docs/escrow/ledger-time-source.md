# Ledger Time Source

Every time-dependent decision in the escrow contract flows through a single
helper function: `utils::now_seconds`. This page documents its semantics,
precision, trust assumptions, every call site that depends on it, and how to
advance time deterministically in tests.

## Overview

```rust
// contracts/escrow/src/utils.rs
pub fn now_seconds(env: &Env) -> u64 {
    env.ledger().timestamp()
}
```

`now_seconds` is a thin wrapper around `env.ledger().timestamp()`. The wrapper
exists so that:

1. **All modules share one canonical time source.** A single grep for
   `now_seconds` shows every place time matters.
2. **Trust and precision assumptions are documented in one place** rather than
   scattered across call sites.
3. **Tests can reason about time** through a well-defined API rather than
   chasing ad-hoc `env.ledger()` calls.

## Precision and trust assumptions

Stellar ledger timestamps are set by network validators and advance at
**roughly 5-second intervals**. This has concrete implications:

| Property | Detail |
| --- | --- |
| **Resolution** | ~5 seconds, not sub-second. A deadline set to `now_seconds(&env) + 1` will likely be satisfied on the very next ledger. Never design fine-grained (sub-minute) deadlines around this value. |
| **Validator skew** | Validators can report timestamps that differ by a small number of seconds from wall-clock time. The ledger timestamp is unsuitable for cryptographic nonce expiry or any protocol that demands exact real-time correspondence. |
| **Monotonicity** | The timestamp never goes backwards across successive ledgers on a given network. |
| **Consensus** | All validators see the same timestamp for a given ledger close. There is no per-node variation. |

### When to use `now_seconds`

- Deadline comparisons (milestone overdue, migration expiry)
- Scheduling relative offsets (e.g. "7 days from now")
- Event timestamps for off-chain indexers

### When NOT to use `now_seconds`

- Sub-minute or sub-second deadlines — ledger resolution is too coarse
- Wall-clock-dependent UI logic — use client-side time instead
- Cryptographic nonce expiry — use sequence numbers or random nonces

## Call sites

### Contract logic (affects state transitions)

| Function | File | What it decides |
| --- | --- | --- |
| `Escrow::is_milestone_overdue` | `lib.rs:993` | Whether `now_seconds(&env) > deadline`, gating timeout refunds in `refund_unreleased_milestones` |

### Informational (event payloads only)

All other `env.ledger().timestamp()` calls in the crate appear in event
publishes. These are **informational** — they never influence a state
transition and are acceptable because they do not affect whether an operation
succeeds or fails.

Examples: `bind_settlement_token`, `initialize`, `release_milestone` event
payloads, `cancel_contract`, `pause`/`unpause`, `activate_emergency_pause`,
`resolve_emergency`, `withdraw_protocol_fees`, `submit_work_evidence`,
governance entrypoints, and migration entrypoints.

## Strict-inequality boundary

`is_milestone_overdue` uses **strictly greater** (`>`):

```rust
now_seconds(&env) > deadline
```

This means:

| `now` vs `deadline` | Overdue? |
| --- | --- |
| `now < deadline` | No |
| `now == deadline` | No |
| `now > deadline` | Yes |

At exactly the deadline the milestone is **not** yet overdue. This prevents a
one-second-early timeout refund and gives the freelancer the full deadline
window.

## Deterministic time control in tests

Soroban's test environment provides `env.ledger().with_mut` to set the ledger
timestamp to any value. Since `now_seconds` reads `env.ledger().timestamp()`,
tests can advance time arbitrarily.

### Setting the timestamp

```rust
use soroban_sdk::testutils::Ledger;

// Set the ledger timestamp to a known value.
env.ledger().with_mut(|li| {
    li.timestamp = 1_000;
});
```

### Advancing time past a deadline

```rust
let deadline = 1_000u64;

// Before deadline — not overdue
env.ledger().with_mut(|li| { li.timestamp = deadline - 1; });
assert!(!client.is_milestone_overdue(&contract_id, &0));

// Exactly at deadline — still not overdue (strict >)
env.ledger().with_mut(|li| { li.timestamp = deadline; });
assert!(!client.is_milestone_overdue(&contract_id, &0));

// One second past — now overdue
env.ledger().with_mut(|li| { li.timestamp = deadline + 1; });
assert!(client.is_milestone_overdue(&contract_id, &0));
```

### Worked example matching `test/timeout_tests.rs`

The test file `contracts/escrow/src/test/timeout_tests.rs` contains a complete
example:

```rust
#[test]
fn is_milestone_overdue_transitions_from_false_to_true() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);
    let (_client_addr, _, id) = create_contract(&env, &client);

    let deadline = 5_000u64;
    set_milestone_deadline_and_released(
        &env, &client.address, id, 0, Some(deadline), false,
    );

    // Phase 1: well before deadline
    set_now(&env, 1_000);
    assert!(!client.is_milestone_overdue(&id, &0));

    // Phase 2: one second before
    set_now(&env, deadline - 1);
    assert!(!client.is_milestone_overdue(&id, &0));

    // Phase 3: exactly at deadline
    set_now(&env, deadline);
    assert!(!client.is_milestone_overdue(&id, &0));

    // Phase 4: one second after — transitions to overdue
    set_now(&env, deadline + 1);
    assert!(client.is_milestone_overdue(&id, &0));

    // Phase 5: far after deadline — still overdue
    set_now(&env, deadline + 100_000);
    assert!(client.is_milestone_overdue(&id, &0));
}
```

The helper `set_now` is a thin wrapper used across timeout tests:

```rust
fn set_now(env: &Env, secs: u64) {
    env.ledger().with_mut(|li| {
        li.timestamp = secs;
    });
}
```

## Guidelines for contributors

1. **Always use `now_seconds(env)`** for time comparisons in contract code.
   Never call `env.ledger().timestamp()` directly in business logic.
2. **Use strict `>` for deadline checks** unless there is a documented reason
   for `>=`.
3. **Never design sub-minute deadlines.** Ledger resolution is ~5 seconds.
4. **In tests, always set the ledger timestamp explicitly.** Do not rely on
   default values for time-dependent assertions.
5. **Test the three-point boundary**: before, at, and after every deadline.
