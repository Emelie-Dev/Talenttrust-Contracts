use super::{assert_contract_error, EscrowFixture};
use crate::{ContractStatus, Error};
use soroban_sdk::{
    symbol_short,
    testutils::Events as _,
    token::{Client as TokenClient, StellarAssetClient},
    IntoVal, Val, Vec,
};

fn assert_deposit_event(
    fixture: &EscrowFixture,
    events: Vec<(soroban_sdk::Address, Vec<Val>, Val)>,
    amount: i128,
    funded_amount: i128,
    total_deposited: i128,
) {
    let expected_topics: Vec<Val> = (symbol_short!("deposit"), fixture.escrow_id)
        .into_val(&fixture.env);
    let expected_data: Val = (amount, funded_amount, total_deposited).into_val(&fixture.env);
    let mut matching_events = 0;

    for event in events.iter() {
        if event.0 == fixture.escrow_address && event.1 == expected_topics {
            matching_events += 1;
            assert_eq!(
                event.2, expected_data,
                "deposit event data must match persisted amounts"
            );
        }
    }

    assert_eq!(
        matching_events, 1,
        "exactly one escrow deposit event must use the dedicated deposit topic"
    );
}

/// A fully-funded fixture records the complete milestone total and custody balance.
#[test]
fn funded_fixture_deposits_the_configured_total() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let token = fixture.settlement_token.as_ref().unwrap();

    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        fixture.total_amount()
    );
}

/// Deposits can be staged while the fixture keeps token custody setup uniform.
#[test]
fn deposit_transitions_from_partially_funded_to_funded() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let partial = total / 2;
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::new(&fixture.env, token).mint(&fixture.client, &total);

    assert!(escrow.deposit_funds(&fixture.escrow_id, &fixture.client, &partial));
    let first_deposit_events = fixture.env.events().all();
    assert_deposit_event(&fixture, first_deposit_events, partial, partial, partial);
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::PartiallyFunded
    );
    let remainder = total - partial;
    assert!(escrow.deposit_funds(&fixture.escrow_id, &fixture.client, &remainder));
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
}

/// A full deposit reports the changed aggregate values in the event payload.
#[test]
fn full_deposit_emits_exact_storage_state_payload() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::new(&fixture.env, token).mint(&fixture.client, &total);

    assert!(escrow.deposit_funds(&fixture.escrow_id, &fixture.client, &total));
    let events = fixture.env.events().all();

    assert_deposit_event(&fixture, events, total, total, total);
}

/// Invalid deposit amounts fail before touching the configured SAC balance.
#[test]
fn deposit_rejects_non_positive_amounts() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    for amount in [0_i128, -1_i128] {
        assert_contract_error(
            escrow.try_deposit_funds(&fixture.escrow_id, &fixture.client, &amount),
            Error::AmountMustBePositive,
        );
    }
}
