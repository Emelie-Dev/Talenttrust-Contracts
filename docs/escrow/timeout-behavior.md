# Escrow Timeout Behavior

No deadline, approval-expiry, timeout evaluation, or timeout-driven dispute
entrypoint is implemented in `contracts/escrow/src/lib.rs`.

The current release path validates only paused state, contract existence,
milestone bounds, duplicate release, and available funded balance.

## Planned

Milestone approval expiry and timeout-driven dispute resolution should be
documented here only after the corresponding public entrypoints and storage
fields land.
