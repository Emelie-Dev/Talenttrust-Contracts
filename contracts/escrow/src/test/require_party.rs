//! Tests for the `require_party` helper extracted in refactor/auth-01-require-party.
//!
//! Coverage matrix:
//!  * client accepted for various entrypoints
//!  * freelancer accepted for various entrypoints
//!  * arbiter accepted for various entrypoints
//!  * stranger (non-party) rejected with `PartyNotAuthorized`
//!  * unknown contract rejected with `ContractNotFound` (loaded before party check)

use super::{assert_contract_error, register_client, total_milestone_amount, EscrowFixtureBuilder};
use crate::{Error, EscrowError, ReleaseAuthorization};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn funded_with_arbiter() -> super::EscrowFixture {
    let builder = EscrowFixtureBuilder::new();
    let client = Address::generate(builder.env());
    let freelancer = Address::generate(builder.env());
    let arbiter = Address::generate(builder.env());
    builder
        .with_participants(client, freelancer, Some(arbiter))
        .funded()
        .build()
}

fn with_settlement_token() -> super::EscrowFixture {
    EscrowFixtureBuilder::new().with_settlement_token().build()
}

// ── cancel_contract ──────────────────────────────────────────────────────

#[test]
fn cancel_contract_client_accepted() {
    let f = EscrowFixtureBuilder::new().funded().build();
    assert!(f.escrow().cancel_contract(&f.escrow_id, &f.client));
}

#[test]
fn cancel_contract_freelancer_accepted() {
    let f = EscrowFixtureBuilder::new().funded().build();
    assert!(f.escrow().cancel_contract(&f.escrow_id, &f.freelancer));
}

#[test]
fn cancel_contract_arbiter_accepted() {
    let f = funded_with_arbiter();
    let arbiter = f.arbiter.as_ref().unwrap();
    assert!(f.escrow().cancel_contract(&f.escrow_id, arbiter));
}

#[test]
fn cancel_contract_stranger_rejected() {
    let f = EscrowFixtureBuilder::new().funded().build();
    let stranger = Address::generate(&f.env);
    let result = f.escrow().try_cancel_contract(&f.escrow_id, &stranger);
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── issue_reputation ─────────────────────────────────────────────────────

#[test]
fn issue_reputation_client_accepted() {
    let f = EscrowFixtureBuilder::new().funded().build();
    for i in 0..3u32 {
        f.escrow()
            .approve_milestone_release(&f.escrow_id, &f.client, &i);
        f.escrow().release_milestone(&f.escrow_id, &f.client, &i);
    }
    let comment = soroban_sdk::String::from_str(&f.env, "Great work!");
    assert!(f
        .escrow()
        .issue_reputation(&f.escrow_id, &f.client, &5, &comment));
}

#[test]
fn issue_reputation_stranger_rejected() {
    let f = EscrowFixtureBuilder::new().funded().build();
    for i in 0..3u32 {
        f.escrow()
            .approve_milestone_release(&f.escrow_id, &f.client, &i);
        f.escrow().release_milestone(&f.escrow_id, &f.client, &i);
    }
    let stranger = Address::generate(&f.env);
    let comment = soroban_sdk::String::from_str(&f.env, "Great work!");
    let result = f
        .escrow()
        .try_issue_reputation(&f.escrow_id, &stranger, &5, &comment);
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── submit_work_evidence ─────────────────────────────────────────────────

#[test]
fn submit_work_evidence_freelancer_accepted() {
    let f = EscrowFixtureBuilder::new().funded().build();
    let evidence = soroban_sdk::String::from_str(&f.env, "QmProof");
    assert!(f
        .escrow()
        .submit_work_evidence(&f.escrow_id, &f.freelancer, &0, &evidence));
}

#[test]
fn submit_work_evidence_client_accepted_as_party() {
    let f = EscrowFixtureBuilder::new().funded().build();
    let evidence = soroban_sdk::String::from_str(&f.env, "QmProof");
    assert!(f
        .escrow()
        .submit_work_evidence(&f.escrow_id, &f.client, &0, &evidence));
}

#[test]
fn submit_work_evidence_stranger_rejected() {
    let f = EscrowFixtureBuilder::new().funded().build();
    let stranger = Address::generate(&f.env);
    let evidence = soroban_sdk::String::from_str(&f.env, "QmProof");
    let result = f
        .escrow()
        .try_submit_work_evidence(&f.escrow_id, &stranger, &0, &evidence);
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── raise_dispute ────────────────────────────────────────────────────────

#[test]
fn raise_dispute_client_accepted() {
    let f = funded_with_arbiter();
    assert!(f.escrow().raise_dispute(&f.escrow_id, &f.client));
}

#[test]
fn raise_dispute_freelancer_accepted() {
    let f = funded_with_arbiter();
    assert!(f.escrow().raise_dispute(&f.escrow_id, &f.freelancer));
}

#[test]
fn raise_dispute_arbiter_accepted() {
    let f = funded_with_arbiter();
    let arbiter = f.arbiter.as_ref().unwrap();
    assert!(f.escrow().raise_dispute(&f.escrow_id, arbiter));
}

#[test]
fn raise_dispute_stranger_rejected() {
    let f = funded_with_arbiter();
    let stranger = Address::generate(&f.env);
    let result = f.escrow().try_raise_dispute(&f.escrow_id, &stranger);
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── resolve_dispute ──────────────────────────────────────────────────────

#[test]
fn resolve_dispute_arbiter_accepted() {
    let f = funded_with_arbiter();
    f.escrow().raise_dispute(&f.escrow_id, &f.client);
    let arbiter = f.arbiter.as_ref().unwrap();
    assert!(f.escrow().resolve_dispute(
        &f.escrow_id,
        arbiter,
        &crate::DisputeResolution::FullRefund,
    ));
}

#[test]
fn resolve_dispute_client_not_arbiter_rejected() {
    let f = funded_with_arbiter();
    f.escrow().raise_dispute(&f.escrow_id, &f.client);
    let result = f.escrow().try_resolve_dispute(
        &f.escrow_id,
        &f.client,
        &crate::DisputeResolution::FullRefund,
    );
    assert_contract_error(result, Error::UnauthorizedRole);
}

#[test]
fn resolve_dispute_stranger_rejected() {
    let f = funded_with_arbiter();
    f.escrow().raise_dispute(&f.escrow_id, &f.client);
    let stranger = Address::generate(&f.env);
    let result = f.escrow().try_resolve_dispute(
        &f.escrow_id,
        &stranger,
        &crate::DisputeResolution::FullRefund,
    );
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── finalize_contract ────────────────────────────────────────────────────

#[test]
fn finalize_contract_client_accepted() {
    let f = EscrowFixtureBuilder::new().funded().build();
    for i in 0..3u32 {
        f.escrow()
            .approve_milestone_release(&f.escrow_id, &f.client, &i);
        f.escrow().release_milestone(&f.escrow_id, &f.client, &i);
    }
    assert!(f.escrow().finalize_contract(&f.escrow_id, &f.client));
}

#[test]
fn finalize_contract_freelancer_accepted() {
    let f = EscrowFixtureBuilder::new().funded().build();
    for i in 0..3u32 {
        f.escrow()
            .approve_milestone_release(&f.escrow_id, &f.client, &i);
        f.escrow().release_milestone(&f.escrow_id, &f.client, &i);
    }
    assert!(f.escrow().finalize_contract(&f.escrow_id, &f.freelancer));
}

#[test]
fn finalize_contract_arbiter_accepted() {
    let f = funded_with_arbiter();
    for i in 0..3u32 {
        f.escrow()
            .approve_milestone_release(&f.escrow_id, &f.client, &i);
        f.escrow().release_milestone(&f.escrow_id, &f.client, &i);
    }
    let arbiter = f.arbiter.as_ref().unwrap();
    assert!(f.escrow().finalize_contract(&f.escrow_id, arbiter));
}

#[test]
fn finalize_contract_stranger_rejected() {
    let f = EscrowFixtureBuilder::new().funded().build();
    for i in 0..3u32 {
        f.escrow()
            .approve_milestone_release(&f.escrow_id, &f.client, &i);
        f.escrow().release_milestone(&f.escrow_id, &f.client, &i);
    }
    let stranger = Address::generate(&f.env);
    let result = f.escrow().try_finalize_contract(&f.escrow_id, &stranger);
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── deposit_funds ────────────────────────────────────────────────────────

#[test]
fn deposit_funds_client_accepted() {
    let f = with_settlement_token();
    let token = f.settlement_token.as_ref().unwrap();
    soroban_sdk::token::StellarAssetClient::new(&f.env, token)
        .mint(&f.client, &total_milestone_amount());
    assert!(f
        .escrow()
        .deposit_funds(&f.escrow_id, &f.client, &total_milestone_amount()));
}

#[test]
fn deposit_funds_freelancer_accepted_as_party() {
    let f = with_settlement_token();
    let token = f.settlement_token.as_ref().unwrap();
    soroban_sdk::token::StellarAssetClient::new(&f.env, token)
        .mint(&f.freelancer, &total_milestone_amount());
    assert!(f
        .escrow()
        .deposit_funds(&f.escrow_id, &f.freelancer, &total_milestone_amount()));
}

#[test]
fn deposit_funds_stranger_rejected() {
    let f = with_settlement_token();
    let stranger = Address::generate(&f.env);
    let result = f
        .escrow()
        .try_deposit_funds(&f.escrow_id, &stranger, &total_milestone_amount());
    assert_contract_error(result, Error::PartyNotAuthorized);
}

// ── Unknown contract ─────────────────────────────────────────────────────

#[test]
fn cancel_contract_unknown_id_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let client = Address::generate(&env);
    let result = escrow.try_cancel_contract(&999, &client);
    assert_contract_error(result, EscrowError::ContractNotFound);
}

#[test]
fn raise_dispute_unknown_id_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let escrow = register_client(&env);
    let client = Address::generate(&env);
    let result = escrow.try_raise_dispute(&999, &client);
    assert_contract_error(result, Error::ContractNotFound);
}
