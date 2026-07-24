import re

with open("contracts/escrow/src/test/client_migration.rs", "r") as f:
    c = f.read()

c = c.replace(
    "let env = fixture.env;",
    "let env = fixture.env.clone();"
)
c = c.replace(
    "let client_addr = fixture.client;",
    "let client_addr = fixture.client.clone();"
)
c = c.replace(
    "let token = fixture.settlement_token.unwrap();",
    "let token = fixture.settlement_token.clone().unwrap();"
)

with open("contracts/escrow/src/test/client_migration.rs", "w") as f:
    f.write(c)
