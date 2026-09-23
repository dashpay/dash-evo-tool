BEGIN TRANSACTION;
CREATE TABLE asset_lock_transaction (
                        tx_id BLOB PRIMARY KEY,
                        transaction_data BLOB NOT NULL,
                        amount INTEGER,
                        instant_lock_data BLOB,
                        chain_locked_height INTEGER,
                        identity_id BLOB,
                        identity_id_potentially_in_creation BLOB,
                        wallet BLOB NOT NULL,
                        network TEXT NOT NULL,
                        FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE SET NULL,
                        FOREIGN KEY (identity_id_potentially_in_creation) REFERENCES identity(id) ON DELETE SET NULL,
                        FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                    );
CREATE TABLE contestant (
                        normalized_contested_name TEXT NOT NULL,
                        identity_id BLOB NOT NULL,
                        name TEXT,
                        votes INTEGER,
                        created_at INTEGER,
                        created_at_block_height INTEGER,
                        created_at_core_block_height INTEGER,
                        document_id BLOB,
                        network TEXT NOT NULL,
                        PRIMARY KEY (normalized_contested_name, identity_id, network),
                        FOREIGN KEY (normalized_contested_name, network) REFERENCES contested_name(normalized_contested_name, network) ON DELETE CASCADE
                    );
CREATE TABLE contested_name (
                        normalized_contested_name TEXT NOT NULL,
                        locked_votes INTEGER,
                        abstain_votes INTEGER,
                        awarded_to BLOB,
                        end_time INTEGER,
                        locked INTEGER NOT NULL DEFAULT 0,
                        last_updated INTEGER,
                        network TEXT NOT NULL,
                        PRIMARY KEY (normalized_contested_name, network)
                    );
CREATE TABLE contract (
                        contract_id BLOB,
                        contract BLOB,
                        alias TEXT,
                        network TEXT NOT NULL,
                        PRIMARY KEY (contract_id, network)
                    );
CREATE TABLE identity (
                        id BLOB PRIMARY KEY,
                        data BLOB,
                        status INTEGER NOT NULL DEFAULT 0,
                        is_local INTEGER NOT NULL,
                        alias TEXT,
                        info TEXT,
                        wallet BLOB,
                        wallet_index INTEGER,
                        identity_type TEXT,
                        network TEXT NOT NULL,
                        CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL) OR (wallet IS NULL AND wallet_index IS NULL)),
                        FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
                    );
INSERT INTO "identity" VALUES(X'3B9DA97EBC06C07C0C0C48695EA5D36CAECE9E19E4944AAE5548691D9A80FAE5',X'003B9DA97EBC06C07C0C0C48695EA5D36CAECE9E19E4944AAE5548691D9A80FAE506000000000000020014A9EF1846DE6CC6A5DAD6B2426053740E4CCE5B8D00010001000100020014A9448477330E75AE4E53CC97C270404B6BD98052000200020002000200147B83198C15A671201E7D3706EFE485904D9FBBAF000300030301000200143F93EC2AF939780305355081FCA1D618D11E71090004000401030101A2A1B4AC6FEF22EA2A1A68E8123644B357875F6B412C18109281C146E7B271BC0E636F6E746163745265717565737400002102DA714F474A0764F27BFB7E285248C3673348F9CC20E7BC1CB3BAD014112B94FF0005000502030101A2A1B4AC6FEF22EA2A1A68E8123644B357875F6B412C18109281C146E7B271BC0E636F6E746163745265717565737400002102C02DC205DF2C5A13C6A55A214F6067305B1728FDDC8AA56F74240B8414DC3A9A00FCEC957594000000000001107075626C69632D64706E732D757365720001116D69677465737466697874757265303933FDDE54EF8DA0010000',2,1,'public-dpns-user',NULL,NULL,NULL,'User','testnet');
INSERT INTO "identity" VALUES(X'05B687978344FA2433B2AA99D41F643E2D8581A789CDC23084889CECA5244EA8',X'0005B687978344FA2433B2AA99D41F643E2D8581A789CDC23084889CECA5244EA802000000030100020114C69A0BDA7DAAAE481BE8DEF95E5F347A1D00A4B400010001060100020114EC83A5F23EC17737F590153EB7EED16F7B2F132700FD7893EDB812F60A000000000002010E7075626C69632D65766F6E6F64650000',2,1,'public-evonode',NULL,NULL,NULL,'Evonode','testnet');
CREATE TABLE identity_order (
            pos INTEGER NOT NULL,
            identity_id BLOB NOT NULL,
            PRIMARY KEY(pos),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        );
CREATE TABLE identity_token_balances (
                token_id BLOB NOT NULL,
                identity_id BLOB NOT NULL,
                balance INTEGER NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY(token_id, identity_id, network),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE,
                FOREIGN KEY (token_id) REFERENCES token(id) ON DELETE CASCADE
             );
CREATE TABLE proof_log (
                        proof_id INTEGER PRIMARY KEY AUTOINCREMENT,
                        request_type INTEGER NOT NULL,
                        request_bytes BLOB NOT NULL,
                        path_query_bytes BLOB NOT NULL,
                        height INTEGER NOT NULL,
                        time_ms INTEGER NOT NULL,
                        proof_bytes BLOB NOT NULL,
                        error TEXT
                    );
CREATE TABLE scheduled_votes (
                identity_id BLOB NOT NULL,
                contested_name TEXT NOT NULL,
                vote_choice TEXT NOT NULL,
                time INTEGER NOT NULL,
                executed INTEGER NOT NULL DEFAULT 0,
                network TEXT NOT NULL,
                PRIMARY KEY (identity_id, contested_name),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
            );
CREATE TABLE settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            password_check BLOB,
            main_password_salt BLOB,
            main_password_nonce BLOB,
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            custom_dash_qt_path TEXT,
            overwrite_dash_conf INTEGER,
            theme_preference TEXT DEFAULT 'System',
            database_version INTEGER NOT NULL
        );
INSERT INTO "settings" VALUES(1,NULL,NULL,NULL,'testnet',0,NULL,NULL,'System',11);
CREATE TABLE token (
                id BLOB PRIMARY KEY,
                token_alias TEXT NOT NULL,
                token_config BLOB NOT NULL,
                data_contract_id BLOB NOT NULL,
                token_position INTEGER NOT NULL,
                network TEXT NOT NULL,
                FOREIGN KEY (data_contract_id, network)
                    REFERENCES contract(contract_id, network)
                    ON DELETE CASCADE
            );
CREATE TABLE token_order (
            pos INTEGER NOT NULL,
            token_id BLOB NOT NULL,
            identity_id BLOB NOT NULL,
            PRIMARY KEY(pos, token_id),
            FOREIGN KEY (token_id) REFERENCES token(id) ON DELETE CASCADE,
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        );
CREATE TABLE top_up (
                identity_id BLOB NOT NULL,
                top_up_index INTEGER NOT NULL,
                amount INTEGER NOT NULL,
                PRIMARY KEY (identity_id, top_up_index),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
            );
CREATE TABLE utxos (
                        txid BLOB NOT NULL,
                        vout INTEGER NOT NULL,
                        address TEXT NOT NULL,
                        value INTEGER NOT NULL,
                        script_pubkey BLOB NOT NULL,
                        network TEXT NOT NULL,
                        PRIMARY KEY (txid, vout, network)
                    );
CREATE TABLE wallet (
                seed_hash BLOB NOT NULL PRIMARY KEY,
                encrypted_seed BLOB NOT NULL,
                salt BLOB NOT NULL,
                nonce BLOB NOT NULL,
                master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
                alias TEXT,
                is_main INTEGER,
                uses_password INTEGER NOT NULL,
                password_hint TEXT,
                network TEXT NOT NULL
            );
CREATE TABLE wallet_addresses (
                seed_hash BLOB NOT NULL,
                address TEXT NOT NULL,
                derivation_path TEXT NOT NULL,
                balance INTEGER,
                path_reference INTEGER NOT NULL,
                path_type INTEGER NOT NULL,
                PRIMARY KEY (seed_hash, address),
                FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            );
CREATE INDEX idx_wallet_addresses_path_reference ON wallet_addresses (path_reference);
CREATE INDEX idx_wallet_addresses_path_type ON wallet_addresses (path_type);
CREATE INDEX idx_utxos_address ON utxos (address);
CREATE INDEX idx_utxos_network ON utxos (network);
CREATE INDEX idx_identity_local_network_type
             ON identity (is_local, network, identity_type);
CREATE INDEX idx_alias_network ON contract (alias, network);
CREATE INDEX idx_proof_log_request_type_time ON proof_log (request_type, time_ms);
CREATE INDEX idx_proof_log_time ON proof_log (time_ms);
CREATE INDEX idx_proof_log_error_request_type_time ON proof_log (error, request_type, time_ms);
CREATE INDEX idx_proof_log_error_time ON proof_log (error, time_ms);
DELETE FROM "sqlite_sequence";
COMMIT;
