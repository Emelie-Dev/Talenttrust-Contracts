# feat(escrow): validate Split dispute amounts and arbiter authorization (#486)

## Summary

This PR closes issue #486 by introducing the missing arbiter-guarded entry points around the dispute resolution flow that was previously implemented as a *pure* `resolution_payouts` helper with no public surface. The gap was: a `Split(client, freelancer)` could be mathematically validated, but there was no contract method that enforced *who* could call it and *when* — i.e. an unauthorized caller (or a caller routing around the `Disputed` lifecycle) could apply a payout.

This PR closes that gap by:

- **`require_auth()` + arbiter check** — only the configured arbiter can apply a resolution; non-arbiter callers surface `UnauthorizedRole` (or, in production before the role branch, a Soroban auth error).
- **State enforcement** — every arbiter action requires the contract to be in `Disputed` status; any other state is rejected with `InvalidState`.
- **Logic reuse** — `resolution_payouts`, `split_payouts`, `final_status_after_resolution` and `final_status_after_split` are pure helpers in a new `dispute` module; the entry points call into them and never restate the math.
- **Event emission** — every dispute lifecycle event is published as `dsp_rais(contract_id)` or `dsp_resl(contract_id)`, the latter carrying `(caller, resolution_code, client_payout, freelancer_payout, timestamp)` so off-chain indexers can reconstruct the arbiter's decision deterministically.
- **Accounting** — `released_amount`/`refunded_amount` are persisted via `safe_add_amounts` and the `AccountingInvariantViolated` invariant is checked before and after every state write.

The `Split` invariant (`client_amount + freelancer_amount == available_balance && both non-negative`) is enforced *before* any state writes happen, so the arbiter cannot corrupt the accounting by submitting an inconsistent split.

## New public API

```rust
// Dispute-aware contract creation. Some(addr) enables the dispute
// lifecycle; None is equivalent to create_contract.
pub fn create_contract_with_arbiter(
    env: Env,
    client: Address,
    freelancer: Address,
    arbiter: Option<Address>,
    milestone_amounts: Vec<i128>,
    deposit_mode: DepositMode,
) -> u32;

// Client/freelancer raises a dispute. Auth restricted to parties;
// requires an arbiter configured at creation; only Funded/PartiallyFunded.
pub fn raise_dispute(
    env: Env,
    contract_id: u32,
    caller: Address,
    reason_hash: BytesN<32>,
) -> bool;

// Arbiter resolves Release | Refund | Cancel.
pub fn resolve_dispute(
    env: Env,
    contract_id: u32,
    caller: Address,
    resolution: DisputeResolution,
) -> bool;

// Arbiter resolves an arbitrary Split(client_amount, freelancer_amount).
// Both components validated pre-state-write.
pub fn resolve_dispute_split(
    env: Env,
    contract_id: u32,
    caller: Address,
    split: DisputeSplit,
) -> bool;

// Read dispute metadata (raiser, reason hash, raised-at timestamp).
pub fn get_dispute(env: Env, contract_id: u32) -> DisputeMetadata;
```

`soroban_sdk::BytesN<32>` is used for `reason_hash` so the off-chain
reason/evidence can be referenced without bringing the entire payload
into contract storage.

## New types

```rust
// Unit-only enum: Soroban contracttype rejects non-unit variants.
#[contracttype]
#[repr(u32)]
pub enum DisputeResolution {
    Release = 0,    // freelancer receives all
    Refund  = 1,    // client receives all
    Cancel  = 2,    // terminate without fund movement
}

// Splits live in a separate struct because the Soroban contracttype
// macro only accepts unit variants on enums; this also keeps the wire
// schema for simple resolutions compact.
#[contracttype]
pub struct DisputeSplit {
    pub client_amount: i128,
    pub freelancer_amount: i128,
}

#[contracttype]
pub struct DisputeMetadata {
    pub raised_by: Address,
    pub reason_hash: BytesN<32>,
    pub raised_at: u64,
}

#[contracttype]
pub enum DataKey {
    // …existing variants…
    Dispute(u32),  // DisputeMetadata keyed per-contract
}
```

## New error variants

| Error | Code | When |
|-------|------|------|
| `DisputeArbiterMissing` | 44 | raise/resolve called on a contract without an arbiter |
| `DisputeNotFound`        | 45 | resolve called without matching `DataKey::Dispute` metadata |

Production-grade `UnauthorizedRole`, `InvalidState`, `NonPositiveAmount`,
and `AccountingInvariantViolated` are reused from the existing
`EscrowError` set.

## Pure helpers (`dispute.rs`)

| Function | Purpose |
|----------|---------|
| `split_payouts(env, contract, split) -> (client_amount, freelancer_amount)` | Validates Split invariants pre-state-write; panics with `NonPositiveAmount` / `AccountingInvariantViolated`. |
| `final_status_after_resolution(contract, resolution) -> ContractStatus` | Computes the post-resolution `ContractStatus` for Release/Refund/Cancel, applying **post-state** accounting (new_released/new_refunded vs milestone total) so a fully-funded `Release` lands on `Completed`, not `Funded`. |
| `final_status_after_split(contract, split) -> ContractStatus` | Same post-state logic for an arbitrary `DisputeSplit`. |
| `require_arbiter(env, contract, caller)` | Auth: contract must have an arbiter; caller must equal it. |
| `require_party(env, contract, caller)` | Auth: caller must be client or freelancer (used by `raise_dispute`). |

## State machine update

| From | To | Trigger |
|------|----|---------|
| `Funded` / `PartiallyFunded` | `Disputed` | `raise_dispute` (client or freelancer only) |
| `Disputed` | `Completed` | arbiter `resolve_dispute(Release)` or `resolve_dispute_split(client=0)` |
| `Disputed` | `Refunded`  | arbiter `resolve_dispute(Refund)`  or `resolve_dispute_split(freelancer=0)` |
| `Disputed` | `Cancelled` | arbiter `resolve_dispute(Cancel)` |
| `Disputed` | `Funded` (mixed) | arbiter `resolve_displit(c, f)` with both non-zero |

While in `Disputed`, direct `release_milestone` calls are rejected with
`InvalidState` so the arbiter remains the sole mover of funds.

## Events

| Topic | Payload | When |
|-------|---------|------|
| `(dsp_rais, contract_id)` | `(caller, reason_hash, timestamp)` | `raise_dispute` succeeded |
| `(dsp_resl, contract_id)` | `(caller, resolution_code, client_payout, freelancer_payout, timestamp)` | `resolve_dispute` and `resolve_dispute_split` succeeded. `resolution_code` ∈ {0=Release, 1=Refund, 2=Cancel, 3=Split}. |
| `(audit, contract_id)`     | `(from_status, to_status, actor, timestamp)` | Existing audit log; fires on every dispute lifecycle transition. |

## Tests (`test/dispute.rs`)

A new 28-test suite in `contracts/escrow/src/test/dispute.rs` covers, with deterministic assertions:

- `raise_dispute` happy paths: client or freelancer can raise on `Funded` and on `PartiallyFunded`; metadata is persisted.
- `raise_dispute` error paths: arbiter cannot raise (`UnauthorizedRole`); third party cannot raise; missing-arbiter contract rejects (`DisputeArbiterMissing`); non-funded contracts reject (`InvalidState`); second raise rejects (`InvalidState`).
- `resolve_dispute` happy paths: `Release` → `Completed` with `released_amount == 300` and `refunded_amount == 0`; `Refund` → `Refunded` with the inverse accounting; `Cancel` → `Cancelled`.
- `resolve_dispute_split` happy paths: 100/200 split persists correct accounting and lands in `Funded` (mixed); 300/0 → `Refunded`; 0/300 → `Completed`.
- `resolve_dispute_split` invariants: 50/100 (sum 150 ≠ 300 available) rejected via `try_*` + `assert_contract_error(EscrowError::AccountingInvariantViolated)`; `-1/301` rejected via `assert_contract_error(NonPositiveAmount)`.
- `resolve_dispute` auth: client / freelancer / outsider cannot resolve; non-disputed contract rejects.
- State blocking: `release_milestone_blocked_in_disputed_state` confirms direct release is blocked once a dispute is raised.
- Storage error path: `get_dispute` panics with `DisputeNotFound` when no metadata exists.
- Pause accountability: `pause_blocks_raise_dispute`, `pause_blocks_resolve_dispute`, `pause_blocks_resolve_dispute_split`.

## Validation

- `cargo fmt --all` — clean
- `cargo check --all-targets` — clean (no warnings)
- `cargo test --all-targets` — **59 passed; 0 failed; 0 ignored; 0 warnings**

## Files changed

| File | Change |
|------|--------|
| `contracts/escrow/src/types.rs` | `DisputeResolution`, `DisputeSplit`, `DisputeMetadata`, `DataKey::Dispute`, `EscrowError::DisputeArbiterMissing` + `DisputeNotFound`, code constants |
| `contracts/escrow/src/dispute.rs` | **new** — pure helpers `split_payouts`, `final_status_after_resolution`, `final_status_after_split`, `require_arbiter`, `require_party` |
| `contracts/escrow/src/lib.rs` | `mod dispute` re-export, `create_contract_with_arbiter`, `raise_dispute`, `resolve_dispute`, `resolve_dispute_split`, `get_dispute`, `Disputed`-state guard in `release_milestone` |
| `contracts/escrow/src/test/mod.rs` | wires `mod dispute;` so the new suite is actually compiled |
| `contracts/escrow/src/test/dispute.rs` | 28 new dispute tests |
| `docs/escrow/README.md` | New §3 *Dispute Resolution Flow* event/state-machine documentation and updated lifecycle, security, and integration example sections |
| `PR_BODY.md` | This document, kept in-repo for review history |

## Notes for reviewers

1. Soroban's `#[contracttype]` macro only accepts unit enum variants, so
   the `Split` payload lives in a separate `DisputeSplit` struct and is
   routed through a dedicated `resolve_dispute_split` entry point. The
   `DisputeResolution` enum itself stays unit-only (Release/Refund/Cancel).
2. The post-state accounting fix (`new_released / new_refunded` compared
   to `sum(milestones)`) is the heart of the state-machine correctness:
   without it, a `Release` resolution on a freshly-funded contract would
   report `Funded` instead of `Completed`. `final_status_after_resolution`
   computes the post-state explicitly.
3. The auth chain in production is `caller.require_auth()` →
   `dispute_require_arbiter`. In tests `mock_all_auths()` makes the
   first step a no-op so the explicit role-check branch is reached; in
   production the Soroban auth error fires *before* `require_arbiter`.
   This is documented in the helper doc-comments.
4. The `create_contract` signature is intentionally unchanged to avoid
   breaking the existing test suite. The new arbiter-aware constructor
   is `create_contract_with_arbiter`. Code duplication with
   `create_contract` is flagged as a follow-up refactor candidate.

## Out of scope / follow-ups

- Factor a private `create_contract_inner` to deduplicate
  `create_contract` and `create_contract_with_arbiter`.
- Extract a private `enter_dispute_resolution_or_panic` helper to
  consolidate the auth/state prelude repeated across `raise_dispute`,
  `resolve_dispute`, and `resolve_dispute_split`.
- Decide whether `raise_dispute` should accept `PartiallyFunded`
  (current: yes) or only `Funded` (current doc: yes) — the two are
  consistent but worth re-confirming with the protocol team.
