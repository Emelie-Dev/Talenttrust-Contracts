# fix: grant pending reputation credit consistently on completion

## Description
This pull request addresses an issue where pending reputation credits were not granted consistently when an escrow contract reaches the `Completed` status. Previously, the canonical entrypoints in `lib.rs` for releasing milestones and resolving disputes did not grant a pending reputation credit, causing `issue_reputation` to panic due to the missing credit.

## Changes
- **Centralized Credit Logic**: Introduced a helper function `grant_pending_reputation_credit` in `lib.rs`.
- **Consistent Accrual**: Updated all three completion paths (`release_milestone`, `refund_unreleased_milestones`, and `resolve_dispute`) in `lib.rs` to call the helper when `contract.status` transitions to `Completed`.
- **Removed Dead Code**: Cleaned up `contracts/escrow/src/dispute.rs` by removing redundant and unused `#[contractimpl]` blocks that implemented out-of-sync logic.
- **Documentation**: Updated `docs/escrow/README.md` to reflect the uniform credit lifecycle for escrow completion.
- **Tests**: Added tests to `contracts/escrow/src/test/reputation.rs` covering completion paths for disputes and partial refunds.

## Validation
- The `issue_reputation` method correctly decrements the `PendingReputationCredits`, preventing double crediting, and successfully handles errors as expected.
- Reputation credits accrue exactly once per complete contract.
- Local tests have been written to ensure `PendingReputationCredits` logic acts predictably and correctly across scenarios.
