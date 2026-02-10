-- Dash Evo Tool — E2E Test Seed Data
--
-- Pre-populates the database with known test state for full E2E tests.
-- Uses testnet network. All hex values are realistic but synthetic test data.
--
-- Run with: sqlite3 dash-evo-tool.db < seed-data.sql

-- =============================
-- Settings (testnet, onboarded)
-- =============================
INSERT OR REPLACE INTO settings (
    id, network, start_root_screen, database_version,
    onboarding_completed, theme_preference, disable_zmq,
    core_backend_mode, user_mode
) VALUES (
    1, 'testnet', 0, 1,
    1, 'System', 1,
    1, 'Advanced'
);

-- =============================
-- Wallets
-- =============================

-- HD Wallet: "Test Wallet Alpha"
-- seed_hash is SHA-256 of a test mnemonic (32 bytes)
INSERT OR REPLACE INTO wallet (
    seed_hash, encrypted_seed, salt, nonce,
    master_ecdsa_bip44_account_0_epk,
    alias, is_main, uses_password, network,
    confirmed_balance, unconfirmed_balance, total_balance
) VALUES (
    X'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2',
    X'0000000000000000000000000000000000000000000000000000000000000000',
    X'1111111111111111111111111111111111111111111111111111111111111111',
    X'222222222222222222222222',
    X'0488b21e0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000',
    'Test Wallet Alpha', 1, 0, 'testnet',
    500000000, 0, 500000000
);

-- HD Wallet addresses (account 0)
INSERT OR REPLACE INTO wallet_addresses (seed_hash, address, derivation_path, balance, path_reference, path_type, total_received) VALUES
    (X'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2', 'yXk4vG2mRqUBbZBvquJkXpGN8LhftVQm3S', 'm/44''/1''/0''/0/0', 250000000, 0, 0, 250000000),
    (X'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2', 'yN7BjdHw3T2RfgqUehMQkWPX9fLv5j8CZy', 'm/44''/1''/0''/0/1', 150000000, 1, 0, 150000000),
    (X'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2', 'yT3WpknxHJfZzU7ENq9d8y2KvN4mA5bGcR', 'm/44''/1''/0''/0/2', 100000000, 2, 0, 100000000),
    (X'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2', 'yR2DeJk9M5cCLbfUn3HvPpA8sWx7qZ4YtE', 'm/44''/1''/0''/1/0', 0, 0, 1, 0);

-- =============================
-- Identities
-- =============================

-- Identity 1: linked to Test Wallet Alpha
INSERT OR REPLACE INTO identity (
    id, data, status, is_local, alias, wallet, wallet_index, identity_type, network
) VALUES (
    X'b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1',
    NULL, 0, 1, 'TestUser1',
    X'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2',
    0, 'user', 'testnet'
);

-- Identity 2: standalone (no wallet link)
INSERT OR REPLACE INTO identity (
    id, data, status, is_local, alias, wallet, wallet_index, identity_type, network
) VALUES (
    X'c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2',
    NULL, 0, 1, 'TestUser2',
    NULL, NULL, 'user', 'testnet'
);

-- Identity ordering
INSERT OR REPLACE INTO identity_order (pos, identity_id) VALUES
    (0, X'b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1'),
    (1, X'c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2');

-- =============================
-- Contracts (system contracts)
-- =============================

-- DPNS contract
INSERT OR REPLACE INTO contract (contract_id, contract, alias, network) VALUES (
    X'e668c659af66aee1e72c186dde7b5b7e0a1d712a09c40d5721f622bf53c53155',
    NULL, 'DPNS', 'testnet'
);

-- DashPay contract
INSERT OR REPLACE INTO contract (contract_id, contract, alias, network) VALUES (
    X'4fac99c9ee7d72b4a85c02a2eedcae53a55a1afe5c89c44a8b46fcdd81caf211',
    NULL, 'DashPay', 'testnet'
);

-- =============================
-- Contested Names (DPNS)
-- =============================

INSERT OR REPLACE INTO contested_name (
    normalized_contested_name, locked_votes, abstain_votes,
    awarded_to, end_time, locked, last_updated, network
) VALUES
    ('alice', 5, 2, NULL, 1735689600, 0, 1735600000, 'testnet'),
    ('bob', 3, 1, NULL, 1735776000, 0, 1735600000, 'testnet');

-- Contestants for 'alice'
INSERT OR REPLACE INTO contestant (
    normalized_contested_name, identity_id, name, votes,
    created_at, document_id, network
) VALUES
    ('alice', X'b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1',
     'alice', 3, 1735500000, X'1111111111111111111111111111111111111111111111111111111111111111', 'testnet'),
    ('alice', X'c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2',
     'alice', 2, 1735500000, X'2222222222222222222222222222222222222222222222222222222222222222', 'testnet');

-- =============================
-- UTXOs
-- =============================

INSERT OR REPLACE INTO utxos (txid, vout, address, value, script_pubkey, network) VALUES
    (X'aabbccdd11223344556677889900aabbccddeeff11223344556677889900aabb', 0,
     'yXk4vG2mRqUBbZBvquJkXpGN8LhftVQm3S', 250000000,
     X'76a91400112233445566778899aabbccddeeff0011223388ac', 'testnet'),
    (X'bbccddee22334455667788990011aabbccddeeff22334455667788990011aabb', 1,
     'yN7BjdHw3T2RfgqUehMQkWPX9fLv5j8CZy', 150000000,
     X'76a91411223344556677889900aabbccddeeff1122334488ac', 'testnet');

-- =============================
-- Scheduled Votes
-- =============================

INSERT OR REPLACE INTO scheduled_votes (
    identity_id, contested_name, vote_choice, time, executed, network
) VALUES (
    X'b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1',
    'alice', 'lock', 1735700000, 0, 'testnet'
);
