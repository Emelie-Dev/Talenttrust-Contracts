use soroban_sdk::Env;

/// Returns the current ledger timestamp in seconds since the Unix epoch.
///
/// This is the **single source of truth** for all time-related operations in the
/// escrow contract. Every consumer — milestone deadline checks, migration expiry,
/// and event timestamps — must read time through this helper rather than calling
/// `env.ledger().timestamp()` directly. Centralising on `now_seconds` gives the
/// codebase a single point to audit for precision and trust assumptions.
///
/// # Precision and trust assumptions
///
/// Stellar ledger timestamps are set by the network validators and advance at
/// roughly 5-second intervals. This means:
///
/// * **Resolution is ~5 seconds**, not sub-second. A deadline set to
///   `now_seconds(&env) + 1` will likely be satisfied on the very next ledger.
///   Never design fine-grained (sub-minute) deadlines around this value.
/// * **Validators can skew** the timestamp by a small number of seconds from
///   wall-clock time. The ledger timestamp is therefore unsuitable for
///   cryptographic nonce expiry or any protocol that demands exact real-time
///   correspondence.
/// * **Monotonicity is guaranteed** within a single network; the timestamp
///   never goes backwards across successive ledgers.
///
/// For these reasons the contract uses **strictly greater** (`>`) comparisons
/// when testing deadlines (see [`Escrow::is_milestone_overdue`]), so the
/// milestone is only considered overdue once the ledger has clearly advanced
/// past the deadline.
///
/// # Call sites
///
/// | Consumer | Module | What it decides |
/// | --- | --- | --- |
/// | [`Escrow::is_milestone_overdue`] | `lib.rs` | Whether a milestone has passed its deadline, gating timeout refunds |
///
/// All other `env.ledger().timestamp()` calls in the crate (event payloads,
/// `finalize` metadata) are **informational** and do not affect contract logic;
/// they are acceptable because they never influence a state transition.
///
/// # Arguments
///
/// * `env` — The Soroban contract environment providing access to the ledger.
///
/// # Returns
///
/// The current ledger close time as a `u64` representing seconds since the
/// Unix epoch (1970-01-01T00:00:00Z).
///
/// # Testing
///
/// In tests, advance time deterministically with `env.ledger().with_mut`:
///
/// ```ignore
/// use soroban_sdk::testutils::Ledger;
///
/// // Set the ledger timestamp to a known value.
/// env.ledger().with_mut(|li| {
///     li.timestamp = 1_000;
/// });
///
/// // Later, advance time past a deadline.
/// env.ledger().with_mut(|li| {
///     li.timestamp = 2_000;
/// });
/// ```
///
/// See `contracts/escrow/src/test/timeout_tests.rs` for a complete worked
/// example covering every branch of `is_milestone_overdue` and the strict-
/// inequality boundary.
pub fn now_seconds(env: &Env) -> u64 {
    env.ledger().timestamp()
}
