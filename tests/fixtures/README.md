# Platform compatibility fixtures

`grovedb-v1.hex` is the unmodified `grovedb_proof` from Platform v4.1.1's
[`document_read_no_contract` vector](https://github.com/dashpay/platform/blob/69b85c81af8e000e8506edaa13406d1f6274af5a/packages/rs-sdk/tests/vectors/document_read_no_contract/msg_GetDataContractRequest_e4cf74168e03a40bd159451456b501c1ba166a2dd8f6efb31b0289dc011da983.json).
It proves absence of contract ID `[0; 32]`, at protocol 12, height 1255.
GroveDB 3.1.0 rejects these bytes with `UnexpectedVariant { found: 1 }`.

`v0.9.3-identities.hex` contains the two unmodified `identity.data` blobs from
[DET historical fixture](https://github.com/dashpay/dash-evo-tool/blob/35fe23bd088595eac7cd32c33f5c3c8d82287261/tests/migration-fixtures/v0.9.3-public-identities/data.sql).
The original writer ran against v0.9.3 (`3268b7366dc6d7a65e8a534d9411330eafbf4763`),
Rust 1.89 and its unchanged lockfile. These are public testnet identities
(`public-dpns-user` and `public-evonode`) with no private keys or wallet associations.
They exercise the original bincode rc.3 encoding, including keys, contract bounds,
aliases and a DPNS name. Tests must never regenerate them with the new codec.
