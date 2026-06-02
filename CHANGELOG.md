# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- **Sign-time wallet unlock**: the passphrase is now requested just-in-time, the
  moment an operation actually needs your secret (sending funds, registering an
  identity, signing). The prompt offers an optional "Keep this wallet unlocked
  until I close the app" checkbox so a busy session asks only once. This replaces
  the old upfront unlock gate that held the wallet open for the whole session —
  the HD seed is no longer kept in memory between operations; it is decrypted on
  demand and wiped as soon as the operation finishes.

- **Wallet storage backend**: HD wallet seeds and single-key private keys are now
  stored in an upstream `platform-wallet-storage` encrypted vault (`secrets.pwsvault`)
  rather than in the legacy `data.db` SQLite database. Wallet metadata (alias, main flag,
  Core wallet name) moves to a new `det-app.sqlite` key-value sidecar. The legacy
  `data.db` file is left intact for safety; it is no longer read at runtime.

- **Cold-start migration**: on the first launch after upgrading, DET automatically
  migrates wallet seeds, metadata, and imported single-key data from `data.db` into
  the new storage layout. A progress banner is shown during migration (typically
  under one second on local storage). The migration is idempotent — subsequent
  launches skip it via a completion sentinel in `det-app.sqlite`.

### Removed

- Proof log screen (internal developer tool, not part of the public feature set).
- QR-code wallet import flow for identity funding and top-up screens.

### Fixed

- `WalletBackend` is now initialised eagerly at `AppState` start, eliminating a
  retry-loop spam on the SDK connection during cold boot.
- Wallet store is rehydrated on cold start, resolving a regression where wallets
  were not visible after the storage migration.
