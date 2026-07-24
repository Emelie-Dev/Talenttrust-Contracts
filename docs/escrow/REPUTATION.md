# Reputation Credential Issuance

The Escrow contract issues reputation credentials (ratings) to freelancers after a contract reaches `Completed` status. A pending-credit system ensures that reputation can only be issued once per completed contract and that the counter never goes negative.

## Pending-Reputation-Credit Lifecycle

`PendingReputationCredits` is a per-freelancer counter that tracks the number of completed contracts awaiting client-issued reputation.

### Increment

A pending credit is **incremented** when a contract transitions to `Completed` status. This happens in three paths:

| Path | Trigger |
|---|---|
| `release_milestone` | All milestones are released or refunded → status becomes `Completed` |
| `refund_unreleased_milestones` | Some milestones released, remaining refunded → status becomes `Completed` |
| `resolve_dispute` | Dispute resolved with partial payout to freelancer → status becomes `Completed` |

If all milestones are refunded (none released), status becomes `Refunded` and no credit is incremented.

### Decrement

A pending credit is **decremented** when `issue_reputation` succeeds. The guard `if pending <= 0 { panic }` ensures the counter never drops below zero. If no credit exists (pending == 0), the call panics with `InvalidState`, preventing underflow.

### Invariant

```
for each freelancer:
    pending_credits == completed_contracts - reputation_issued_count
    pending_credits >= 0          // never negative
    completed_contracts >= 0      // never negative
    reputation_issued_count >= 0  // never negative
```

## Validation Rules

1. **Client authorization:** Only the contract client may call `issue_reputation`. Unauthorized callers fail with `UnauthorizedRole`.
2. **Comment validation:** The comment must not be empty (`EmptyComment`) and must not exceed 200 characters (`CommentTooLong`).
3. **Self-rating prevention:** If `contract.client == contract.freelancer`, issuance fails with `SelfRating`. This guards against degenerate contracts.
4. **Contract completion gating:** Reputation can only be issued after the contract is `Completed`. Non-completed contracts fail with `NotCompleted`.
5. **Rating bounds:** Ratings must be between `1` and `5` inclusive. Values outside this range fail with `InvalidRating`.
6. **Duplicate issuance protection:** Reputation may only be issued once per contract. Subsequent attempts fail with `ReputationAlreadyIssued`.
7. **Pending-credit guard:** `issue_reputation` panics with `InvalidState` if `PendingReputationCredits(freelancer) <= 0`. This ensures the counter is never decremented below zero.

## Reputation Aggregation

Successful issuance updates the freelancer's aggregate `ReputationRecord`:

- `completed_contracts` increments by `1`
- `total_rating` increases by the rating value
- `last_rating` is set to the most recent rating

Pending reputation credits are also decremented on success.

## Test Coverage

The escrow test suite covers the following scenarios in `contracts/escrow/src/test/reputation.rs`:

### Negative paths
- unauthorized caller
- self-rating when client equals freelancer (`SelfRating`)
- non-completed contract
- invalid rating bounds
- duplicate issuance
- empty comment
- comment too long

### Pending-credit lifecycle
- credit incremented on contract completion (release path)
- credit incremented on mixed refund/release completion
- no credit incremented for fully refunded contracts
- credit consumed on reputation issuance
- multiple contracts accumulate credits for the same freelancer
- credits consumed one per issuance
- pending credits never go negative (guard test)

### Reputation aggregation
- verified reputation record update with `completed_contracts`, `total_rating`, `last_rating`
- `get_pending_reputation_credits` returns accurate values

## Average Rating Accessor

The contract exposes `get_average_rating(freelancer) -> Option<i128>` as a read-only helper for consumer convenience. The returned integer is scaled by 10,000, so `40_000` represents an average rating of `4.0`.

- Returns `None` when the freelancer has no completed contracts.
- Returns `Some(value)` when `completed_contracts > 0`.
- The result is computed as `(total_rating * 10_000) / completed_contracts` using checked arithmetic.

## Security Assumptions

- **Access Control:** `issue_reputation` requires client authentication.
- **Self-rating invariant:** A single principal cannot both issue and receive reputation on the same contract (`SelfRating` when `client == freelancer`).
- **Contract Completion:** Only `Completed` contracts are eligible for reputation issuance.
- **Duplicate issuance guard:** Repeat issuance is blocked by the `ReputationIssued` field on the contract struct.
- **Pending-credit invariant:** `pending >= 0` is enforced by the `issue_reputation` guard. Increments only happen on legitimate completion transitions, decrements only on successful issuance.
- **Aggregate consistency:** Reputation totals and pending credits are updated atomically within each entrypoint.
