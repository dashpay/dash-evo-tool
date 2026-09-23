# Public historical identity fixture

Generator: fixture-writer.rs, compiled as an example inside dash-evo-tool tag v0.9.3 (3268b7366dc6d7a65e8a534d9411330eafbf4763) using Rust 1.89 and the unchanged historical Cargo.lock. Database schema initialization and QualifiedIdentity bincode encoding come from that checkout. No private keys or wallet ownership associations are supplied.

Public explorer snapshots downloaded 2026-09-17:

- user.json: https://testnet.platform-explorer.pshenmic.dev/identity/51iVVoMU8ANbxDv7wySh5Sk6mZGSEMpCkeqtkr24mHdE
- masternode.json: https://testnet.platform-explorer.pshenmic.dev/identity/PJUBWbXWmzEYCs99rAAbnCiHRzrnhKLQrXbmSsuPBYB
- validator.json: https://testnet.platform-explorer.pshenmic.dev/validators?limit=1
- documents.json: https://testnet.platform-explorer.pshenmic.dev/identity/51iVVoMU8ANbxDv7wySh5Sk6mZGSEMpCkeqtkr24mHdE/documents
- domain.json: https://testnet.platform-explorer.pshenmic.dev/document/6TST6y5sktpeN9ts6FLidnVHjjhUaZqTW7V5cw3e1QDs
- domain-transaction.json: https://testnet.platform-explorer.pshenmic.dev/transaction/908845682B350F1001E514045429981289CF7A11C09FE6365522B8504ECE99DA

DPNS uses the historical label-only representation. The acquired_at timestamp is the explorer's domain creation timestamp (corroborated by its successful creation transaction and identity alias), 2026-09-11T00:47:47.422Z. The explorer does not expose a separate $createdAt field; revision 1 and DOCUMENT_CREATE are checked. Public preorder salt/entropy and transaction signature in snapshots are published chain data, not signing secrets.

The fixture represents locally imported public identities. Local aliases and active status are fixture metadata. It does not establish possession of the public identity's private keys. The masternode has real public OWNER and TRANSFER keys; its Evonode classification is corroborated by validator.json proTxInfo.type=Evo with the same identity. No voter/operator association is invented.

Reproduce: copy fixture-writer.rs to examples/fixture-writer.rs in the pinned checkout, then run `cargo +1.89 run --locked --example fixture-writer -- /path/to/snapshots /path/to/new-output-directory`. Source directory must contain user.json, masternode.json, domain.json and validator.json; output data.db must not already exist. The executable asserts an old-codec round trip and exactly two identities, zero wallets.

Export the resulting database with `sqlite3 /path/to/new-output-directory/data.db .dump > data.sql`. The harness disables foreign keys while restoring the dump and checks them afterward. The checked-in SQL is fixed historical input; CI never regenerates it with current code.
