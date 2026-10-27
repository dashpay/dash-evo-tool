# Pinned stored-contract bytes

Data-contract bytes exactly as Dash Evo Tool writes them to its per-network
k/v store (`StoredContract::contract_bytes`, from
`DataContract::serialize_to_bytes_with_platform_version`). The test
`contracts_saved_by_an_earlier_build_still_load` in
`src/context/contract_token_db.rs` decodes them with the current build.

- `p<N>-<label>.hex`: the contract written at protocol version `N`, hex-encoded.
- Labels, in the order of the test helper `stored_contracts_at`: `user` (a
  user contract with a unique index and a token), `dpns`, `dashpay`,
  `token-history` (system contracts at that protocol version).

Provenance: written by the `dpp` of dashpay/platform
`575d961fe0c54bd7e01a3cf3823494e58c6c4fdb`, the platform pin of DET
`ff59f84df4`, before the bump to the stored contract format of
dashpay/platform#5578.

These bytes are a compatibility record. Never regenerate them to make the test
pass: a failure means saved contracts would no longer load after an upgrade.
Add files written by a newer build alongside them instead.
