# Escrow Security Notes

This document reflects the escrow API currently implemented in `contracts/escrow/src/lib.rs`.

## Implemented Controls

- `initialize(admin)` is single-use and requires `admin.require_auth()`.
- Pause and emergency controls require the stored admin's authorization.
- Mutating lifecycle calls fail while paused or in emergency mode.
- `create_contract` requires client authorization, rejects identical
  client/freelancer addresses, rejects empty milestones, caps milestone count
  to `MAX_MILESTONES` (10), and validates milestone amounts using centralized
  amount validation. Validation enforces: positivity, minimum positive amount of
  1 stroop, maximum single amount of 1,000,000,000,0000000 stroops (1M tokens),
  and safe accumulation of the total milestone amount with checked arithmetic to
  prevent overflow. The total is validated against the governed `max_escrow_total_stroops`
  or `i128::MAX` if unset.
- `deposit_funds` validates the deposit amount using centralized amount validation
  (enforcing positivity and maximum single amount limits). Crucially, it safely
  accumulates the total of all milestones using checked arithmetic (`accumulate_amounts`)
  to prevent panic on overflow—a defense-in-depth measure against the scenario where
  a contract with many large milestones could brick if the total calculation panicked
  during funding. The deposit is then validated to ensure it does not exceed the
  accumulated total, and rejects repeat exact-total deposits, exact-total mismatches,
  and incremental overfunding.
- `release_milestone` requires `caller.require_auth()`, enforces the contract's
  `ReleaseAuthorization` mode (ClientOnly, ArbiterOnly, ClientAndArbiter, or
  MultiSig), and checks valid non-expired approvals before releasing funds.
  MultiSig requires both client and freelancer approvals via `check_approvals`,
  and release may be triggered only by the stored client or freelancer.
- `issue_reputation` requires the stored client as caller, matching freelancer,
  completed status, rating in `1..=5`, and no prior reputation issuance for the
  contract.
- `cancel_contract` requires client or freelancer authorization and rejects
  completed or already-cancelled contracts.
- `finalize_contract` enforces the following guards in evaluation order:
  1. **require_not_paused** — rejects with `ContractPaused` when the pause flag
     is active; rejects with `EmergencyActive` when the emergency flag is set
     and the pause flag is not (the check inspects `Paused` before `Emergency`,
     so activating the emergency pause sets both flags and surfaces
     `ContractPaused`).
  2. **finalizer.require_auth()** — the caller must authorize the transaction.
  3. **load_contract** — rejects with `ContractNotFound` for an unknown
     `contract_id`.
  4. **require_not_finalized** — rejects with `AlreadyFinalized` if a
     `FinalizationRecord` already exists for the contract.
  5. **require_finalizer_role** — rejects with `UnauthorizedRole` unless
     `finalizer` is the stored client, freelancer, or assigned arbiter.
  6. **status check** — rejects with `InvalidStatusTransition` unless the
     contract status is `Completed` or `Disputed`. All other statuses
     (`Created`, `Funded`, `PartiallyFunded`, `Cancelled`, `Refunded`) are
     rejected.

  After passing all guards, an immutable `FinalizationRecord` is written to
  persistent storage and a `finalized` event is emitted. All subsequent
  contract-specific mutating calls (`release_milestone`,
  `refund_unreleased_milestones`, `cancel_contract`, and a repeated
  `finalize_contract`) then fail with `AlreadyFinalized`.
- Aggregate amount math uses checked helpers where totals are accumulated.
- Balance-changing operations verify the core accounting invariant:
  `total_deposited == released_amount + refunded_amount + available_balance`.
- Dispute payout resolution preserves the available balance across all supported
  branches: `FullRefund`, `FullPayout`, `PartialRefund`, and `Split` always
  return non-negative payouts whose sum equals the available balance, and the
  `Split` branch rejects any combination that would create or destroy value.
- Finalization summaries use checked arithmetic and persistent storage. They do
  not expire through TTL and do not create, deduct, or withdraw protocol fees.

## Milestone Total Validation

The contract enforces the following bounds on milestone amounts:

- **Maximum milestone count:** 10 milestones per contract (`MAX_MILESTONES`).
- **Individual milestone:** Each milestone amount must be in the range `[1, 1_000_000_0000000]` stroops.
- **Total milestone sum:** The sum of all milestone amounts must not exceed `max_escrow_total_stroops` 
  (typically 1,000,000,0000000 stroops, the same as `MAX_TOTAL_ESCROW_STROOPS`).
- **Safe accumulation:** Both `create_contract` and `deposit_funds` use the `accumulate_amounts` 
  helper with checked arithmetic to compute the total. This prevents overflow panics and ensures that
  any contract created with valid milestones will not panic during funding operations. If a total
  were somehow created that exceeds `i128::MAX`, the accumulation would fail with `PotentialOverflow`
  instead of panicking.

## Known Live Gaps

- The contract records escrow accounting only. Token custody, token transfers, and atomic asset movement are managed outside `lib.rs` and must be handled by a separate audited integration contract or protocol suite.
- Secure two-step admin state transfer and standalone public protocol fee extraction/withdrawal are not implemented as public entrypoints.
- `ReadinessChecklist.governed_params_set` exists, but no live governance parameter setter entrypoint updates it to `true`.

## Planned Security Work

- Two-step admin transfer: [#318](https://github.com/Talenttrust/Talenttrust-Contracts/issues/318)
- Protocol fee extraction/withdrawal interface: [#314](https://github.com/Talenttrust/Talenttrust-Contracts/issues/314)
- Governed parameter setter/readiness wiring: [#323](https://github.com/Talenttrust/Talenttrust-Contracts/issues/323)
- Structured deposit and fee events: [#336](https://github.com/Talenttrust/Talenttrust-Contracts/issues/336)
- Canonical storage-key reference: [#342](https://github.com/Talenttrust/Talenttrust-Contracts/issues/342)

## Reviewer Checklist

1. Verify no integration guide treats planned entrypoints as live API.
2. Verify pause/emergency blocks every mutating lifecycle call.
3. Verify duplicate release, duplicate reputation issuance, overfunding, and
   invalid amount paths fail closed.
4. Verify off-chain token transfer integrations are atomic or idempotent with
   respect to escrow state changes.
## Refund Gating

`refund_unreleased_milestones` rejects calls when:
- A finalization record exists for the contract (`AlreadyFinalized`).
- The contract status is not `Created`, `Funded`, or `Disputed` (`InvalidState`).

This prevents a client from requesting refunds against a cancelled, completed,
or already-finalized contract.

## Terminal-state matrix (value-moving operations)

The following states are terminal for all value-moving entrypoints (`deposit_funds`,
`release_milestone`, `refund_unreleased_milestones`):

- `Cancelled` — The contract has been cancelled and any further deposits,
  releases, or refunds are rejected with `ContractCancelled`.
- `Refunded` — The contract has been fully refunded and further value movement
  is rejected with `ContractRefunded`.

These explicit errors were introduced to make lifecycle audits clearer and to
prevent ambiguous `InvalidState` errors from masking terminal-state violations.

## finalize_contract Guard Matrix (issue #709)

`finalize_contract(contract_id, finalizer)` enforces guards in the order listed
below. Each guard is independently tested in
`contracts/escrow/src/test/security.rs`.

| # | Guard | Error returned | Condition |
|---|-------|----------------|-----------|
| 1 | `require_not_paused` — pause flag | `ContractPaused` | `DataKey::Paused` is `true` |
| 2 | `require_not_paused` — emergency flag | `EmergencyActive` | `DataKey::Emergency` is `true` and `Paused` is `false`; note that `activate_emergency_pause()` sets both flags so guard 1 fires first |
| 3 | `finalizer.require_auth()` | Soroban auth failure (panics) | Caller did not sign the transaction |
| 4 | `load_contract` | `ContractNotFound` | No `Contract` stored for `contract_id` |
| 5 | `require_not_finalized` | `AlreadyFinalized` | `DataKey::Finalization(contract_id)` already exists |
| 6 | `require_finalizer_role` | `UnauthorizedRole` | `finalizer` ≠ stored client, freelancer, or arbiter |
| 7 | status check | `InvalidStatusTransition` | Status ∉ `{Completed, Disputed}` |

### Status gate — terminal states that must be rejected

| Status | Expected error |
|--------|----------------|
| `Created` | `InvalidStatusTransition` |
| `Funded` | `InvalidStatusTransition` |
| `PartiallyFunded` | `InvalidStatusTransition` |
| `Cancelled` | `InvalidStatusTransition` |
| `Refunded` | `InvalidStatusTransition` |
| `Completed` | ✅ Allowed |
| `Disputed` | ✅ Allowed |

### Post-finalize mutation block

After a `FinalizationRecord` is written, `require_not_finalized` is called at
the entry of every contract-specific mutating entrypoint:

| Entrypoint | Error when finalized |
|------------|---------------------|
| `finalize_contract` (repeat) | `AlreadyFinalized` |
| `release_milestone` | `AlreadyFinalized` |
| `refund_unreleased_milestones` | `AlreadyFinalized` |
| `cancel_contract` | `AlreadyFinalized` |

Read-only queries (`get_contract`, `get_finalization_record`, `get_reputation`,
etc.) are never blocked by the finalization record.
