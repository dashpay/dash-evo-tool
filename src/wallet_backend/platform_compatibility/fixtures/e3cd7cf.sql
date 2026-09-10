-- Materialized empty schema from dashpay/platform e3cd7cf. No user data.
-- Includes the V011 Rust conversion hook dropping legacy pool tables.
PRAGMA application_id=1347180372;
CREATE TABLE "account_registrations" (
    wallet_id BLOB NOT NULL,
    account_type TEXT NOT NULL CHECK (account_type IN ('standard', 'standard_bip44', 'standard_bip32', 'coinjoin', 'identity_registration', 'identity_topup', 'identity_topup_unbound', 'identity_invitation', 'asset_lock_address_topup', 'asset_lock_shielded_topup', 'provider_voting', 'provider_owner', 'provider_operator', 'provider_platform', 'dashpay_receiving', 'dashpay_external', 'platform_payment')),
    account_index INTEGER NOT NULL,
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
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
);
CREATE TABLE core_sync_state (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    last_processed_height INTEGER,
    synced_height INTEGER, chainlock_height INTEGER, last_applied_chain_lock BLOB,
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
    spent_in_txid BLOB, winner_mined_height INTEGER, is_sweep_placeholder INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (wallet_id, outpoint),
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
        FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
    );
CREATE TABLE identities (
    identity_id BLOB NOT NULL PRIMARY KEY,
    wallet_id BLOB,
    identity_index INTEGER,
    entry_blob BLOB NOT NULL,
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
);
CREATE TABLE "identity_keys" (
    wallet_id BLOB,
    identity_id BLOB NOT NULL,
    key_id INTEGER NOT NULL,
    public_key_blob BLOB NOT NULL,
    public_key_hash BLOB NOT NULL,
    -- Reserved for a future typed projection; always NULL today.
    -- derivation_indices lives inside public_key_blob (the IdentityKeyWire
    -- blob is the single source of truth).
    derivation_blob BLOB,
    PRIMARY KEY (identity_id, key_id),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE,
    FOREIGN KEY (wallet_id, identity_id)
        REFERENCES identities(wallet_id, identity_id) ON DELETE CASCADE
);
CREATE TABLE identity_scan_failed_indices (
    wallet_id BLOB NOT NULL,
    failed_index INTEGER NOT NULL CHECK (failed_index >= 0),
    PRIMARY KEY (wallet_id, failed_index),
    FOREIGN KEY (wallet_id) REFERENCES identity_scan_states(wallet_id) ON DELETE CASCADE
);
CREATE TABLE identity_scan_states (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    complete INTEGER NOT NULL CHECK (complete IN (0, 1)),
    probed_from INTEGER NOT NULL CHECK (probed_from >= 0),
    probed_through INTEGER NOT NULL CHECK (probed_through >= probed_from),
    unlocated_gap INTEGER NOT NULL CHECK (unlocated_gap IN (0, 1)),
    -- A scan cannot both have answered everything and be sitting on a gap
    -- nobody could name.
    CHECK (complete = 0 OR unlocated_gap = 0),
    FOREIGN KEY (wallet_id) REFERENCES wallets(wallet_id) ON DELETE CASCADE
);
CREATE TABLE ignored_senders (
    wallet_id BLOB NOT NULL,
    owner_id BLOB NOT NULL,
    sender_id BLOB NOT NULL,
    ignored_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (wallet_id, owner_id, sender_id),
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
        FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
);
CREATE TABLE platform_address_sync (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    sync_height INTEGER NOT NULL,
    sync_timestamp INTEGER NOT NULL,
    last_known_recent_block INTEGER NOT NULL,
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
);
CREATE TABLE platform_addresses (
    wallet_id BLOB NOT NULL,
    account_index INTEGER NOT NULL,
    address_index INTEGER NOT NULL,
    address BLOB NOT NULL,
    balance INTEGER NOT NULL,
    nonce INTEGER NOT NULL, as_of_height INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (wallet_id, address),
    FOREIGN KEY (wallet_id) REFERENCES "wallets"(wallet_id) ON DELETE CASCADE
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
CREATE TABLE "wallets" (
    wallet_id BLOB NOT NULL PRIMARY KEY,
    network TEXT NOT NULL CHECK (network IN ('mainnet', 'testnet', 'devnet', 'regtest')),
    birth_height INTEGER NOT NULL
);
CREATE INDEX idx_contacts_owner ON contacts(owner_id);
CREATE INDEX idx_core_address_pool_script
    ON core_address_pool(wallet_id, script);
CREATE INDEX idx_core_address_pool_used
    ON core_address_pool(wallet_id, used);
CREATE INDEX idx_core_transactions_height ON core_transactions(wallet_id, height);
CREATE INDEX idx_core_utxos_spent ON core_utxos(wallet_id, spent);
CREATE INDEX idx_core_utxos_unmaterialized ON core_utxos(wallet_id, winner_mined_height)
    WHERE is_sweep_placeholder = 1;
CREATE INDEX idx_identities_wallet ON identities(wallet_id);
CREATE UNIQUE INDEX idx_identities_wallet_identity ON identities(wallet_id, identity_id);
CREATE INDEX idx_identity_keys_wallet_identity ON identity_keys(wallet_id, identity_id);
CREATE INDEX idx_ignored_senders_owner ON ignored_senders(owner_id);
CREATE INDEX idx_pending_contact_crypto_owner ON pending_contact_crypto(owner_identity_id);
CREATE TRIGGER cascade_children_on_identity_delete
AFTER DELETE ON identities
FOR EACH ROW
BEGIN
    DELETE FROM identity_keys   WHERE identity_id = OLD.identity_id;
    DELETE FROM contacts        WHERE owner_id    = OLD.identity_id;
    DELETE FROM ignored_senders WHERE owner_id    = OLD.identity_id;
    DELETE FROM pending_contact_crypto WHERE owner_identity_id = OLD.identity_id;
END;
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
AFTER DELETE ON "wallets"
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
CREATE TRIGGER setnull_core_utxos_on_tx_delete AFTER DELETE ON core_transactions
BEGIN
    UPDATE core_utxos SET spent_in_txid = NULL
    WHERE wallet_id = OLD.wallet_id AND spent_in_txid = OLD.txid;
END;
INSERT INTO refinery_schema_history VALUES (1, 'initial', '2026-09-10T00:00:00Z', '16458714021387590417');
INSERT INTO refinery_schema_history VALUES (2, 'address_height_pin', '2026-09-10T00:00:00Z', '9304453063389616695');
INSERT INTO refinery_schema_history VALUES (3, 'invitations', '2026-09-10T00:00:00Z', '6768710306020099565');
INSERT INTO refinery_schema_history VALUES (4, 'asset_lock_recovered_status', '2026-09-10T00:00:00Z', '14446754812616312642');
INSERT INTO refinery_schema_history VALUES (5, 'dpns_name_states', '2026-09-10T00:00:00Z', '16834970739141546284');
INSERT INTO refinery_schema_history VALUES (6, 'tracked_masternodes', '2026-09-10T00:00:00Z', '13718514320935649081');
INSERT INTO refinery_schema_history VALUES (7, 'utxo_sweep_winner_height', '2026-09-10T00:00:00Z', '4410976868559573281');
INSERT INTO refinery_schema_history VALUES (8, 'rehydration_base_schema', '2026-09-10T00:00:00Z', '8883371305674850298');
INSERT INTO refinery_schema_history VALUES (9, 'unified', '2026-09-10T00:00:00Z', '8389295906542582065');
INSERT INTO refinery_schema_history VALUES (10, 'pool_public_key', '2026-09-10T00:00:00Z', '15088205568906581463');
INSERT INTO refinery_schema_history VALUES (11, 'pool_reserved_at', '2026-09-10T00:00:00Z', '1184102594786438566');
INSERT INTO refinery_schema_history VALUES (12, 'drop_core_utxo_metadata', '2026-09-10T00:00:00Z', '14281733277552006158');
INSERT INTO refinery_schema_history VALUES (13, 'shielded_viewing_keys', '2026-09-10T00:00:00Z', '2886147678304763509');
INSERT INTO refinery_schema_history VALUES (14, 'single_source_core_confirmation_height', '2026-09-10T00:00:00Z', '13462228277166821110');
INSERT INTO refinery_schema_history VALUES (15, 'purge_legacy_empty_script_spent_utxos', '2026-09-10T00:00:00Z', '723175730717787103');
INSERT INTO refinery_schema_history VALUES (16, 'identity_keys_null_scope_requires_existing_identity', '2026-09-10T00:00:00Z', '5573841766625796742');
INSERT INTO refinery_schema_history VALUES (17, 'identity_scan_state', '2026-09-10T00:00:00Z', '14086773559107496341');
INSERT INTO refinery_schema_history VALUES (18, 'identity_hard_delete', '2026-09-10T00:00:00Z', '14764815300602197850');
INSERT INTO meta_store_generation VALUES (0, X'22222222222222222222222222222222');
