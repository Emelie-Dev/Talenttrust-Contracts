//! Named constants used throughout the escrow contract.
//!
//! Extracting literal numbers into documented constants makes the code
//! self-describing and prevents accidental inconsistencies.

/// Maximum basis points (= 100 %), the highest possible protocol fee.
///
/// All fee rates are expressed as basis points (1 bps = 0.01 %).  A value
/// of `10_000` represents 100 % of the escrowed amount.
pub const MAX_BPS: u32 = 10_000;

/// Denominator for basis-point arithmetic (10 000 bps = 100 %).
///
/// Fee calculations multiply the amount by the fee in bps and then divide
/// by `BPS_DENOMINATOR` to obtain the fee in stroops:
///
/// ```ignore
/// fee = amount * fee_bps / BPS_DENOMINATOR
/// ```
pub const BPS_DENOMINATOR: u32 = 10_000;

// ── Rating bounds ──────────────────────────────────────────────────────────

/// Minimum valid reputation rating (inclusive).
///
/// Ratings outside the [1, 5] range are rejected with `Error::InvalidRating`.
pub const MIN_RATING: u32 = 1;

/// Maximum valid reputation rating (inclusive).
pub const MAX_RATING: u32 = 5;

// ── Size limits ────────────────────────────────────────────────────────────

/// Maximum byte length of a reputation feedback comment.
///
/// Comments longer than this are rejected with `Error::CommentTooLong`.
pub const MAX_COMMENT_BYTES: u32 = 200;

/// Maximum byte length of work evidence submitted with a dispute.
///
/// Evidence exceeding this limit is rejected with `Error::EvidenceTooLong`.
pub const MAX_EVIDENCE_BYTES: u32 = 256;

// ── Dispute partial-refund split ───────────────────────────────────────────

/// Numerator for the freelancer's share in a partial refund (30 %).
///
/// In a `PartialRefund` resolution the freelancer receives
/// `available * PARTIAL_REFUND_FREELANCER_SHARE / PARTIAL_REFUND_DENOMINATOR`
/// and the client receives the remainder.
pub const PARTIAL_REFUND_FREELANCER_SHARE: i128 = 30;

/// Denominator for partial-refund percentage calculation.
pub const PARTIAL_REFUND_DENOMINATOR: i128 = 100;

// ── Contract ID allocation ─────────────────────────────────────────────────

/// The first contract ID allocated by the system.
///
/// `DataKey::NextContractId` is initialised to this value in `initialize`
/// and returned when no contract has yet been created.
pub const INITIAL_CONTRACT_ID: u32 = 1;

// ── Reputation credits ─────────────────────────────────────────────────────

/// Unit increment for pending reputation credits.
///
/// Each completed contract grants one pending credit; each `issue_reputation`
/// call consumes one.  The unit value is always `1`.
pub const REPUTATION_CREDIT_INCREMENT: i128 = 1;
