-- Materialized empty schema from dashpay/platform 67d4ef3. No user data.
PRAGMA application_id=1347180372;
CREATE TABLE account_registrations (
    wallet_id BLOB NOT NULL,
    account_type TEXT NOT NULL CHECK (account_type IN ('standard_bip44', 'standard_bip32', 'coinjoin', 'identity_registration', 'identity_topup', 'identity_topup_unbound', 'identity_invitation', 'asset_lock_address_topup', 'asset_lock_shielded_topup', 'provider_voting', 'provider_owner', 'provider_operator', 'provider_platform', 'dashpay_receiving', 'dashpay_external', 'platform_payment')),
    account_index INTEGER NOT NULL,
    -- Discriminators sharing (account_type, account_index) across distinct
    -- accounts: PlatformPayment key_class and the DashPay (user, friend)
    -- identity pair. Sentinel default for variants without that axis.
    key_class INTEGER NOT NULL DEFAULT 0,
    user_identity_id BLOB NOT NULL DEFAULT (zeroblob(32)),
    friend_identity_id BLOB NOT NULL DEFAULT (zeroblob(32)),
    account_xpub_bytes BLOB NOT NULL,
    PRIMARY KEY (wallet_id, account_type, account_index, key_class, user_identity_id, friend_identity_id),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE "asset_locks" (
    wallet_id BLOB NOT NULL,
    outpoint BLOB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('built', 'broadcast', 'is_locked', 'chain_locked', 'consumed', 'recovered_from_chain')),
    account_index INTEGER NOT NULL,
    identity_index INTEGER NOT NULL,
    amount_duffs INTEGER NOT NULL,
    lifecycle_blob BLOB NOT NULL,
    PRIMARY KEY (wallet_id, outpoint),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE contacts (
    wallet_id BLOB NOT NULL,
    owner_id BLOB NOT NULL,
    contact_id BLOB NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('sent', 'received', 'established')),
    outgoing_request BLOB,
    incoming_request BLOB,
    alias TEXT,
    note TEXT,
    is_hidden INTEGER,
    accepted_accounts BLOB,
    -- G1c: set when external-account registration permanently fails for a
    -- contact (so the sync sweep stops retrying a poisoned channel);
    -- cleared on a superseding rotation. Nullable — readers treat NULL as
    -- `false`.
    payment_channel_broken INTEGER,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (wallet_id, owner_id, contact_id),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE core_address_pool (
    wallet_id BLOB NOT NULL,
    account_type TEXT NOT NULL,
    account_index INTEGER NOT NULL,
    key_class INTEGER NOT NULL DEFAULT 0,
    user_identity_id BLOB NOT NULL DEFAULT (zeroblob(32)),
    friend_identity_id BLOB NOT NULL DEFAULT (zeroblob(32)),
    pool_type INTEGER NOT NULL CHECK (pool_type IN (0, 1, 2, 3)),
    address_index INTEGER NOT NULL,
    script BLOB NOT NULL,
    used INTEGER NOT NULL DEFAULT 0 CHECK (used IN (0, 1)), public_key BLOB NULL, key_type INTEGER NULL CHECK (key_type IS NULL OR key_type IN (0, 1, 2)), reserved_at INTEGER NULL,
    PRIMARY KEY (wallet_id, account_type, account_index, key_class, user_identity_id, friend_identity_id, pool_type, address_index),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE core_instant_locks (
    wallet_id BLOB NOT NULL,
    txid BLOB NOT NULL,
    islock_blob BLOB NOT NULL,
    PRIMARY KEY (wallet_id, txid),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE core_sync_state (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    last_processed_height INTEGER,
    synced_height INTEGER,
    -- Bincode-encoded `dashcore::ephemerealdata::chain_lock::ChainLock`.
    -- NULL until the first ChainLock has been applied and flushed.
    last_applied_chain_lock BLOB,
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE "core_transactions" (
    wallet_id BLOB NOT NULL,
    txid BLOB NOT NULL,
    height INTEGER,
    block_hash BLOB,
    block_time INTEGER,
    finalized INTEGER NOT NULL,
    record_blob BLOB,
    PRIMARY KEY (wallet_id, txid),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE core_utxos (
    wallet_id BLOB NOT NULL,
    outpoint BLOB NOT NULL,
    value INTEGER NOT NULL,
    script BLOB NOT NULL,
    spent INTEGER NOT NULL,
    PRIMARY KEY (wallet_id, outpoint),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE dashpay_payments_overlay (
    identity_id BLOB NOT NULL,
    payment_id TEXT NOT NULL,
    overlay_blob BLOB NOT NULL,
    PRIMARY KEY (identity_id, payment_id),
    FOREIGN KEY (identity_id) REFERENCES identities(identity_id) ON DELETE CASCADE
);
CREATE TABLE dashpay_profiles (
    identity_id BLOB NOT NULL PRIMARY KEY,
    profile_blob BLOB NOT NULL,
    FOREIGN KEY (identity_id) REFERENCES identities(identity_id) ON DELETE CASCADE
);
CREATE TABLE dpns_name_states (
        wallet_id BLOB NOT NULL,
        document_id BLOB NOT NULL,
        identity_id BLOB NOT NULL,
        label TEXT NOT NULL,
        normalized_label TEXT NOT NULL,
        normalized_parent_domain TEXT NOT NULL,
        price INTEGER CHECK (price IS NULL OR price >= 0),
        status TEXT NOT NULL CHECK (status IN ('owned', 'sold', 'transferred')),
        counterparty_id BLOB,
        created_at_ms INTEGER,
        updated_at_ms INTEGER,
        transferred_at_ms INTEGER,
        last_synced_at_ms INTEGER NOT NULL,
        PRIMARY KEY (wallet_id, document_id),
        CHECK ((status = 'owned') = (counterparty_id IS NULL)),
        FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
    );
CREATE TABLE identities (
    identity_id BLOB NOT NULL PRIMARY KEY,
    wallet_id BLOB,
    identity_index INTEGER,
    entry_blob BLOB NOT NULL,
    tombstoned INTEGER NOT NULL,
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE identity_keys (
    -- NULLABLE: NULL is the canonical `owned by no wallet`, matching
    -- `identities.wallet_id`. Read the two FKs below with that in mind —
    -- SQLite's default MATCH SIMPLE skips foreign-key enforcement
    -- entirely when ANY column of the child key is NULL, so for a
    -- NULL-scoped row BOTH FKs are dormant and neither constrains which
    -- identity the key names. The two triggers after this table exist
    -- precisely to replace that dormancy; the FKs alone are NOT
    -- sufficient, and a NULL-scoped row is only as safe as the triggers.
    wallet_id BLOB,
    identity_id BLOB NOT NULL,
    key_id INTEGER NOT NULL,
    public_key_blob BLOB NOT NULL,
    public_key_hash BLOB NOT NULL,
    -- Reserved for a future typed projection; always NULL today.
    -- derivation_indices lives inside public_key_blob (the
    -- IdentityKeyWire blob is the single source of truth).
    derivation_blob BLOB,
    -- `wallet_id` is deliberately NOT part of the key. `identities`
    -- keys on `identity_id` ALONE, so an identity has exactly one row
    -- and exactly one owning wallet — the scope column here carries no
    -- discriminating power, it is a denormalised copy of
    -- `identities.wallet_id`. The wider `(wallet_id, identity_id,
    -- key_id)` key was the enabling condition for the duplicate-row
    -- corruption: it let the same key exist twice under two scopes, a
    -- state the domain (`IdentityKeysChangeSet`, keyed
    -- `(identity_id, key_id)`) cannot express. Narrowing the key makes
    -- that row pair unrepresentable rather than merely rejected.
    PRIMARY KEY (identity_id, key_id),
    -- Belt-and-braces: the compound FK below already implies a live
    -- `wallets` row (the matched `identities` row carries this same
    -- non-NULL wallet_id and is itself FK'd to `wallets`). Kept as an
    -- explicit statement of intent and a second cascade path.
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE,
    -- Compound: a key may only be filed under the wallet that OWNS the
    -- identity. The single-column form allowed a key to name an identity
    -- parented to a different wallet — a row the per-wallet reader can
    -- never resolve, surfacing much later as a fatal OrphanedIdentityEntry.
    FOREIGN KEY (wallet_id, identity_id)
        REFERENCES identities(wallet_id, identity_id) ON DELETE CASCADE
);
CREATE TABLE ignored_senders (
    wallet_id BLOB NOT NULL,
    owner_id BLOB NOT NULL,
    sender_id BLOB NOT NULL,
    ignored_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (wallet_id, owner_id, sender_id),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE invitations (
        wallet_id BLOB NOT NULL,
        outpoint BLOB NOT NULL,
        status TEXT NOT NULL CHECK (status IN ('created', 'claimed', 'reclaimed')),
        funding_index INTEGER NOT NULL,
        amount_duffs INTEGER NOT NULL,
        expiry_unix INTEGER NOT NULL,
        created_at_secs INTEGER NOT NULL,
        has_inviter INTEGER NOT NULL,
        PRIMARY KEY (wallet_id, outpoint),
        FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
    );
CREATE TABLE meta_contact (
    wallet_id  BLOB NOT NULL,
    owner_id   BLOB NOT NULL,
    contact_id BLOB NOT NULL,
    key        TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    value      BLOB NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (wallet_id, owner_id, contact_id, key)
);
CREATE TABLE meta_data_versions (
    wallet_id BLOB NOT NULL,
    domain TEXT NOT NULL,
    seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (wallet_id, domain)
);
CREATE TABLE meta_global (
    key        TEXT NOT NULL PRIMARY KEY CHECK (length(key) BETWEEN 1 AND 128),
    value      BLOB NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE meta_identity (
    identity_id BLOB NOT NULL,
    key         TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    value       BLOB NOT NULL,
    updated_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (identity_id, key)
);
CREATE TABLE meta_platform_address (
    wallet_id  BLOB NOT NULL,
    address    BLOB NOT NULL,
    key        TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    value      BLOB NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (wallet_id, address, key)
);
CREATE TABLE meta_store_generation (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 0),
    generation BLOB NOT NULL
);
CREATE TABLE meta_token (
    identity_id BLOB NOT NULL,
    token_id    BLOB NOT NULL,
    key         TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    value       BLOB NOT NULL,
    updated_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (identity_id, token_id, key)
);
CREATE TABLE meta_wallet (
    wallet_id  BLOB NOT NULL,
    key        TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    value      BLOB NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (wallet_id, key)
);
CREATE TABLE pending_contact_crypto (
    wallet_id BLOB NOT NULL,
    owner_identity_id BLOB NOT NULL,
    contact_id BLOB NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('register_receiving', 'register_external', 'contact_info_decrypt', 'auto_accept')),
    payload BLOB NOT NULL,
    enqueued_at_ms INTEGER NOT NULL,
    PRIMARY KEY (wallet_id, owner_identity_id, contact_id, kind),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE platform_address_sync (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    sync_height INTEGER NOT NULL,
    sync_timestamp INTEGER NOT NULL,
    last_known_recent_block INTEGER NOT NULL,
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE platform_addresses (
    wallet_id BLOB NOT NULL,
    account_index INTEGER NOT NULL,
    address_index INTEGER NOT NULL,
    address BLOB NOT NULL,
    balance INTEGER NOT NULL,
    nonce INTEGER NOT NULL, as_of_height INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (wallet_id, address),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE refinery_schema_history(
             version int4 PRIMARY KEY,
             name VARCHAR(255),
             applied_on VARCHAR(255),
             checksum VARCHAR(255));
CREATE TABLE shielded_viewing_keys (
    wallet_id BLOB NOT NULL,
    account_index INTEGER NOT NULL CHECK (account_index BETWEEN 0 AND 4294967295),
    viewing_key BLOB NOT NULL,
    PRIMARY KEY (wallet_id, account_index),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE token_balances (
    identity_id BLOB NOT NULL,
    token_id BLOB NOT NULL,
    balance INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (identity_id, token_id),
    FOREIGN KEY (identity_id) REFERENCES identities(identity_id) ON DELETE CASCADE
);
CREATE TABLE tracked_masternodes (
        network TEXT NOT NULL CHECK (network IN ('mainnet', 'testnet', 'devnet', 'regtest')),
        pro_tx_hash BLOB NOT NULL CHECK (length(pro_tx_hash) = 32),
        label TEXT,
        added_at INTEGER NOT NULL,
        snapshot_json TEXT NOT NULL,
        PRIMARY KEY (network, pro_tx_hash)
    );
CREATE TABLE wallets (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    network TEXT NOT NULL CHECK (network IN ('mainnet', 'testnet', 'devnet', 'regtest')),
    birth_height INTEGER NOT NULL
);
CREATE INDEX idx_core_address_pool_script
    ON core_address_pool(wallet_id, script);
CREATE INDEX idx_core_address_pool_used
    ON core_address_pool(wallet_id, used);
CREATE INDEX idx_core_transactions_height ON core_transactions(wallet_id, height);
CREATE INDEX idx_core_utxos_spent ON core_utxos(wallet_id, spent);
CREATE INDEX idx_identities_wallet ON identities(wallet_id);
CREATE UNIQUE INDEX idx_identities_wallet_identity ON identities(wallet_id, identity_id);
CREATE INDEX idx_identity_keys_wallet_identity ON identity_keys(wallet_id, identity_id);
CREATE TRIGGER cascade_meta_contact_on_contact_delete
AFTER DELETE ON contacts
FOR EACH ROW
BEGIN
    DELETE FROM meta_contact
        WHERE wallet_id = OLD.wallet_id
          AND owner_id = OLD.owner_id
          AND contact_id = OLD.contact_id;
END;
CREATE TRIGGER cascade_meta_data_versions_on_wallet_delete
AFTER DELETE ON wallets
FOR EACH ROW
BEGIN
    DELETE FROM meta_data_versions WHERE wallet_id = OLD.wallet_id;
END;
CREATE TRIGGER cascade_meta_on_identity_delete
AFTER DELETE ON identities
FOR EACH ROW
BEGIN
    DELETE FROM meta_identity WHERE identity_id = OLD.identity_id;
    DELETE FROM meta_token    WHERE identity_id = OLD.identity_id;
END;
CREATE TRIGGER cascade_meta_on_wallet_delete
AFTER DELETE ON wallets
FOR EACH ROW
BEGIN
    DELETE FROM meta_wallet           WHERE wallet_id = OLD.wallet_id;
    DELETE FROM meta_contact          WHERE wallet_id = OLD.wallet_id;
    DELETE FROM meta_platform_address WHERE wallet_id = OLD.wallet_id;
END;
CREATE TRIGGER cascade_meta_platform_address_on_address_delete
AFTER DELETE ON platform_addresses
FOR EACH ROW
BEGIN
    DELETE FROM meta_platform_address
        WHERE wallet_id = OLD.wallet_id AND address = OLD.address;
END;
CREATE TRIGGER cascade_meta_token_on_token_balance_delete
AFTER DELETE ON token_balances
FOR EACH ROW
BEGIN
    DELETE FROM meta_token
        WHERE identity_id = OLD.identity_id AND token_id = OLD.token_id;
END;
CREATE TRIGGER identity_keys_null_scope_requires_unowned_identity
BEFORE INSERT ON identity_keys
FOR EACH ROW WHEN NEW.wallet_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'identity_keys.wallet_id is NULL but the named identity is missing or wallet-owned')
    WHERE NOT EXISTS (
        SELECT 1 FROM identities i
        WHERE i.identity_id = NEW.identity_id AND i.wallet_id IS NULL
    );
END;
CREATE TRIGGER identity_keys_null_scope_requires_unowned_identity_on_update
BEFORE UPDATE ON identity_keys
FOR EACH ROW WHEN NEW.wallet_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'identity_keys.wallet_id is NULL but the named identity is missing or wallet-owned')
    WHERE NOT EXISTS (
        SELECT 1 FROM identities i
        WHERE i.identity_id = NEW.identity_id AND i.wallet_id IS NULL
    );
END;
INSERT INTO refinery_schema_history VALUES (1, 'initial', '2026-09-10T00:00:00Z', '17047616343160871465');
INSERT INTO refinery_schema_history VALUES (2, 'address_height_pin', '2026-09-10T00:00:00Z', '9304453063389616695');
INSERT INTO refinery_schema_history VALUES (3, 'unified', '2026-09-10T00:00:00Z', '14737834287968956387');
INSERT INTO refinery_schema_history VALUES (4, 'invitations', '2026-09-10T00:00:00Z', '10484261583046002776');
INSERT INTO refinery_schema_history VALUES (5, 'pool_public_key', '2026-09-10T00:00:00Z', '1184376266715578615');
INSERT INTO refinery_schema_history VALUES (6, 'pool_reserved_at', '2026-09-10T00:00:00Z', '11787317857065590354');
INSERT INTO refinery_schema_history VALUES (7, 'drop_core_utxo_metadata', '2026-09-10T00:00:00Z', '8136185229636655136');
INSERT INTO refinery_schema_history VALUES (8, 'shielded_viewing_keys', '2026-09-10T00:00:00Z', '1296229231248300117');
INSERT INTO refinery_schema_history VALUES (9, 'single_source_core_confirmation_height', '2026-09-10T00:00:00Z', '18418537157120884496');
INSERT INTO refinery_schema_history VALUES (10, 'asset_lock_recovered_status', '2026-09-10T00:00:00Z', '2416860902556468199');
INSERT INTO refinery_schema_history VALUES (11, 'dpns_name_states', '2026-09-10T00:00:00Z', '6272036451822234550');
INSERT INTO refinery_schema_history VALUES (12, 'purge_legacy_empty_script_spent_utxos', '2026-09-10T00:00:00Z', '5078144809172324082');
INSERT INTO refinery_schema_history VALUES (13, 'tracked_masternodes', '2026-09-10T00:00:00Z', '989798159842040942');
INSERT INTO refinery_schema_history VALUES (14, 'identity_keys_null_scope_requires_existing_identity', '2026-09-10T00:00:00Z', '9865513418054625997');
INSERT INTO meta_store_generation VALUES (0, X'11111111111111111111111111111111');
