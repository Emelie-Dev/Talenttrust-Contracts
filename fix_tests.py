import re

with open("contracts/escrow/src/test/client_migration.rs", "r") as f:
    c = f.read()

# Fix migration_blocked_on_refunded_contract
c = re.sub(
    r"fn migration_blocked_on_refunded_contract\(\) \{\n.*?let client = register_client\(&env\);\n\n    let \(client_addr, _freelancer_addr, id\) = create_contract\(&env, &client\);\n    let new_client = Address::generate\(&env\);\n\n    client\.deposit_funds\(&id, &client_addr, &total_milestone_amount\(\)\);",
    """fn migration_blocked_on_refunded_contract() {
    let fixture = crate::test::EscrowFixture::builder().funded().build();
    let env = fixture.env;
    let client = fixture.escrow();
    let id = fixture.escrow_id;
    let client_addr = fixture.client;
    let new_client = Address::generate(&env);""",
    c, flags=re.DOTALL
)

# Fix migration_allowed_on_partially_funded_status
c = re.sub(
    r"fn migration_allowed_on_partially_funded_status\(\) \{\n.*?let client = register_client\(&env\);\n\n    let \(client_addr, _freelancer_addr, id\) = create_contract\(&env, &client\);\n\n    // Deposit less than the full milestone total → PartiallyFunded\n    client\.deposit_funds\(&id, &client_addr, &super::MILESTONE_ONE\);",
    """fn migration_allowed_on_partially_funded_status() {
    let fixture = crate::test::EscrowFixture::builder().with_settlement_token().build();
    let env = fixture.env;
    let client = fixture.escrow();
    let id = fixture.escrow_id;
    let client_addr = fixture.client;
    
    let total = super::MILESTONE_ONE;
    let token = fixture.settlement_token.unwrap();
    soroban_sdk::token::StellarAssetClient::new(&env, &token).mint(&client_addr, &total);

    // Deposit less than the full milestone total → PartiallyFunded
    client.deposit_funds(&id, &client_addr, &super::MILESTONE_ONE);""",
    c, flags=re.DOTALL
)

# Fix migration_allowed_on_funded_status
c = re.sub(
    r"fn migration_allowed_on_funded_status\(\) \{\n.*?let client = register_client\(&env\);\n\n    let \(client_addr, _freelancer_addr, id\) = create_contract\(&env, &client\);\n    client\.deposit_funds\(&id, &client_addr, &total_milestone_amount\(\)\);",
    """fn migration_allowed_on_funded_status() {
    let fixture = crate::test::EscrowFixture::builder().funded().build();
    let env = fixture.env;
    let client = fixture.escrow();
    let id = fixture.escrow_id;
    let client_addr = fixture.client;
    let new_client = Address::generate(&env);""",
    c, flags=re.DOTALL
)

with open("contracts/escrow/src/test/client_migration.rs", "w") as f:
    f.write(c)

