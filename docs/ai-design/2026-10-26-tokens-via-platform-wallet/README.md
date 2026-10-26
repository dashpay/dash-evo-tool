# Token actions through platform-wallet

## Decision

All twelve token state transitions (mint, transfer, burn, freeze, unfreeze,
destroy frozen funds, pause, resume, claim, set price, purchase, settings
update) go through the upstream `platform-wallet` crate,
`IdentityWallet::token_*_with_signer`. DET no longer builds, signs, or
broadcasts them with the SDK builders.

- **Seam:** `src/wallet_backend/tokens.rs`. `WalletBackend::token_actor` returns
  a `TokenActor`, which pairs the upstream `PlatformWallet` holding the identity
  with the acting `QualifiedIdentity` (the signer). It has one method per action.
- **Holding wallet:** the identity's DET wallet link
  (`AppContext::stored_identity_wallet_link`). DET never borrows another
  wallet's handle.
  - No link → `TaskError::TokenActionNeedsWallet`, raised before signing.
  - Link, but the wallet is not registered upstream → the existing
    `WalletNotLoaded` / `WalletLockedNotLoaded` errors.
- **Why `_with_signer`, not `_with_external_signer`:** the external-signer
  variants pick the signing key themselves, take no `StateTransitionCreationOptions`
  (so the Developer signing override and fee agreement would be lost), fetch the
  contract again, and need the identity inside the wallet's `IdentityManager`.
- **Errors:** `_with_signer` returns a raw `dash_sdk::Error`, so each `TokenActor`
  method maps it with `TaskError::from_token_operation(kind, e)`.
  - Proof failures are logged.
  - Dedicated consensus mappings take priority.
  - Unclassified failures become `TokenOperationFailed`; unclassified rejections
    become `TokenOperationRejected`. Both name the operation.
- **UI:** `AppContext::token_action_availability` feeds My Tokens ("View only",
  with transaction buttons disabled) and every token action screen (the reason
  is shown instead of the form). Read-only token data stays visible.
- **Mint recipient:** upstream drops a chosen recipient when the token does not
  allow choosing one. `model::token::MintRecipient` applies the same rule up
  front, so the confirmation names the identity that actually receives the
  tokens.

## Upstream gaps (pin 063589bf)

1. **No wallet-independent token handle.** An identity no wallet holds cannot
   act (an accepted product decision).
2. **`_with_signer` returns `dash_sdk::Error`.** DET labels the operation itself.
3. **The mint recipient is normalised silently** (only a `debug!` log upstream).
4. **No passthrough for `PutSettings` / `user_fee_increase`.** DET does not use
   them for tokens.
5. **`ConfigUpdateResult::GroupActionWithDocument` requires a document.**
   A group settings-update proposal whose proof carries no document is reported
   as a proof error, even if Platform accepted it. DET's earlier raw broadcast
   accepted any proof result. This needs a backend-e2e group test to confirm.

## Not changed

- Contract registration and all token queries.
- Balances, which upstream's `IdentitySyncManager` already owns.
- Post-broadcast bookkeeping, apart from the mint recipient.

Known existing issue (out of scope): the `HistoricalDocument` branches write the
document's `amount` (a delta) as the identity's balance for mint and burn. The
transfer, claim and purchase branches read field names the token-history
contract does not have, so they do nothing.
