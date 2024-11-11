use crate::database::Database;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{
    AddressInfo, ClosedWalletSeed, DerivationPathReference, DerivationPathType, Wallet, WalletSeed,
};
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::bip32::{DerivationPath, ExtendedPubKey};
use dash_sdk::dpp::dashcore::consensus::deserialize;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    InstantLock, Network, OutPoint, ScriptBuf, Transaction, TxOut, Txid,
};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use rusqlite::params;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

impl Database {
    /// Insert a new wallet into the wallet table
    pub fn store_wallet(&self, wallet: &Wallet, network: &Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();

        // Serialize the extended public keys
        let master_ecdsa_bip44_account_0_epk_bytes =
            wallet.master_bip44_ecdsa_extended_public_key.encode();

        self.execute(
            "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint, network)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                wallet.seed_hash(),
                wallet.encrypted_seed_slice(),
                wallet.salt(),
                wallet.nonce(),
                master_ecdsa_bip44_account_0_epk_bytes,
                wallet.alias.clone(),
                wallet.is_main as i32,
                wallet.uses_password,
                wallet.password_hint().clone(),
                network_str
            ],
        )?;
        Ok(())
    }

    /// Update the alias of a wallet based on the seed.
    /// If the alias is `None`, it sets the alias to NULL in the database.
    pub fn set_wallet_alias(
        &self,
        seed_hash: &[u8; 32],
        new_alias: Option<String>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE wallet SET alias = ? WHERE seed_hash = ?",
            params![new_alias, seed_hash],
        )?;

        Ok(())
    }

    /// Update only the alias and is_main fields of a wallet
    pub fn update_wallet_alias_and_main(
        &self,
        seed_hash: &[u8; 32],
        new_alias: Option<String>,
        is_main: bool,
    ) -> rusqlite::Result<()> {
        self.execute(
            "UPDATE wallet SET alias = ?, is_main = ? WHERE seed_hash = ?",
            params![new_alias, is_main as i32, seed_hash],
        )?;
        Ok(())
    }

    /// Add a new address to a wallet with optional balance.
    /// If the address already exists, it does nothing.
    pub fn add_address_if_not_exists(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        derivation_path: &DerivationPath,
        path_reference: DerivationPathReference,
        path_type: DerivationPathType,
        balance: Option<u64>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        // Step 1: Check if the address already exists for the given seed.
        let mut stmt = conn.prepare(
            "SELECT COUNT(1) FROM wallet_addresses
         WHERE seed_hash = ? AND address = ?",
        )?;
        let count: u32 =
            stmt.query_row(params![seed_hash, address.to_string()], |row| row.get(0))?;

        // Step 2: If the address doesn't exist, insert it.
        if count == 0 {
            conn.execute(
                "INSERT INTO wallet_addresses
             (seed_hash, address, derivation_path, path_reference, path_type, balance)
             VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    seed_hash,
                    address.to_string(),
                    derivation_path.to_string(),
                    path_reference as u32,
                    path_type.bits(),
                    balance,
                ],
            )?;
        }
        Ok(())
    }

    /// Update the balance of an existing address.
    pub fn update_address_balance(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        new_balance: u64,
    ) -> rusqlite::Result<()> {
        let rows_affected = self.execute(
            "UPDATE wallet_addresses
         SET balance = ?
         WHERE seed_hash = ? AND address = ?",
            params![new_balance, seed_hash, address.to_string()],
        )?;

        if rows_affected == 0 {
            Err(rusqlite::Error::QueryReturnedNoRows)
        } else {
            Ok(())
        }
    }

    /// Add a balance to an existing address.
    pub fn add_to_address_balance(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        additional_balance: u64,
    ) -> rusqlite::Result<()> {
        let rows_affected = self.execute(
            "UPDATE wallet_addresses
         SET balance = balance + ?
         WHERE seed_hash = ? AND address = ?",
            params![additional_balance, seed_hash, address.to_string()],
        )?;

        if rows_affected == 0 {
            Err(rusqlite::Error::QueryReturnedNoRows)
        } else {
            Ok(())
        }
    }

    /// Retrieve all wallets for a specific network, including their addresses, balances, and known addresses.
    pub fn get_wallets(&self, network: &Network) -> rusqlite::Result<Vec<Wallet>> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();

        // Step 1: Retrieve all wallets for the given network.
        let mut stmt = conn.prepare(
            "SELECT seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint FROM wallet WHERE network = ?",
        )?;

        let mut wallets_map: BTreeMap<[u8; 32], Wallet> = BTreeMap::new();

        let wallet_rows = stmt.query_map([network_str.clone()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let encrypted_seed: Vec<u8> = row.get(1)?;
            let salt: Vec<u8> = row.get(2)?;
            let nonce: Vec<u8> = row.get(3)?;
            let master_ecdsa_bip44_account_0_epk_bytes: Vec<u8> = row.get(4)?;
            let alias: Option<String> = row.get(5)?;
            let is_main: bool = row.get(6)?;
            let uses_password: bool = row.get(7)?;
            let password_hint: Option<String> = row.get(8)?;

            // Reconstruct the extended public keys
            let master_ecdsa_extended_public_key =
                ExtendedPubKey::decode(&master_ecdsa_bip44_account_0_epk_bytes)
                    .expect("Failed to decode ExtendedPubKey");

            let seed_hash_array: [u8; 32] =
                seed_hash.try_into().expect("Seed hash should be 32 bytes");

            // Insert a new Wallet into the map
            wallets_map.insert(
                seed_hash_array,
                Wallet {
                    wallet_seed: WalletSeed::Closed(ClosedWalletSeed {
                        seed_hash: seed_hash_array,
                        encrypted_seed,
                        salt,
                        nonce,
                        password_hint,
                    }),
                    uses_password,
                    master_bip44_ecdsa_extended_public_key: master_ecdsa_extended_public_key,
                    address_balances: BTreeMap::new(),
                    known_addresses: BTreeMap::new(),
                    watched_addresses: BTreeMap::new(),
                    unused_asset_locks: vec![],
                    alias,
                    identities: HashMap::new(),
                    utxos: HashMap::new(),
                    is_main,
                },
            );
            Ok(())
        })?;

        // Collect any errors during wallet row processing
        for wallet in wallet_rows {
            wallet?;
        }

        // Step 2: Retrieve all addresses, balances, and derivation paths associated with the wallets.
        let mut address_stmt = conn.prepare(
            "SELECT seed_hash, address, derivation_path, balance, path_reference, path_type FROM wallet_addresses",
        )?;

        let address_rows = address_stmt.query_map([], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let address: String = row.get(1)?;
            let derivation_path: String = row.get(2)?;
            let balance: Option<u64> = row.get(3)?;
            let path_reference: u32 = row.get(4)?;
            let path_type: u32 = row.get(5)?;

            let seed_hash_array: [u8; 32] =
                seed_hash.try_into().expect("Seed hash should be 32 bytes");
            let address = Address::from_str(&address)
                .expect("Invalid address format")
                .assume_checked();
            let derivation_path = DerivationPath::from_str(&derivation_path)
                .expect("Expected to convert to derivation path");

            // Convert u32 to DerivationPathReference safely
            let path_reference =
                DerivationPathReference::try_from(path_reference).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(std::fmt::Error),
                    )
                })?;

            let path_type = DerivationPathType::from_bits_truncate(path_type as u32);

            Ok((
                seed_hash_array,
                address,
                derivation_path,
                balance,
                path_reference,
                path_type,
            ))
        })?;

        // Step 3: Add addresses, balances, and known addresses to the corresponding wallets.
        for row in address_rows {
            let (seed_array, address, derivation_path, balance, path_reference, path_type) = row?;

            if let Some(wallet) = wallets_map.get_mut(&seed_array) {
                // Update the address balance if available.
                if let Some(balance) = balance {
                    wallet.address_balances.insert(address.clone(), balance);
                }

                // Add the address to the `known_addresses` map.
                wallet
                    .known_addresses
                    .insert(address.clone(), derivation_path.clone());

                // Add the address to the `watched_addresses` map with AddressInfo.
                let address_info = AddressInfo {
                    address: address.clone(),
                    path_reference,
                    path_type,
                };
                wallet
                    .watched_addresses
                    .insert(derivation_path, address_info);
            }
        }

        // Step 4: Retrieve UTXOs for each wallet and add them to the wallets.
        let mut utxo_stmt = conn.prepare(
            "SELECT txid, vout, address, value, script_pubkey FROM utxos WHERE network = ?",
        )?;

        let utxo_rows = utxo_stmt.query_map([network_str.clone()], |row| {
            let txid: Vec<u8> = row.get(0)?;
            let vout: i64 = row.get(1)?;
            let address: String = row.get(2)?;
            let value: i64 = row.get(3)?;
            let script_pubkey: Vec<u8> = row.get(4)?;

            let address = Address::from_str(&address)
                .expect("Invalid address format")
                .assume_checked();

            let outpoint = OutPoint {
                txid: Txid::from_slice(&txid).expect("Invalid txid"),
                vout: vout as u32,
            };
            let tx_out = TxOut {
                value: value as u64,
                script_pubkey: ScriptBuf::from_bytes(script_pubkey),
            };
            Ok((address, outpoint, tx_out))
        })?;

        // Step 5: Add the UTXOs to the corresponding wallets.
        for row in utxo_rows {
            let (address, outpoint, tx_out) = row?;

            for wallet in wallets_map.values_mut() {
                if wallet.known_addresses.contains_key(&address) {
                    wallet
                        .utxos
                        .entry(address.clone())
                        .or_insert_with(HashMap::new)
                        .insert(outpoint, tx_out.clone());
                }
            }
        }

        // Step 6: Retrieve asset lock transactions for each wallet and add them to the wallets.
        let mut asset_lock_stmt = conn.prepare(
            "SELECT wallet, amount, transaction_data, instant_lock_data, chain_locked_height FROM asset_lock_transaction where identity_id IS NULL AND network = ?",
        )?;

        let asset_lock_rows = asset_lock_stmt.query_map([network.to_string()], |row| {
            let wallet_seed: Vec<u8> = row.get(0)?;
            let amount: Duffs = row.get(1)?;
            let tx_data: Vec<u8> = row.get(2)?;
            let islock_data: Option<Vec<u8>> = row.get(3)?;
            let chain_locked_height: Option<CoreBlockHeight> = row.get(4)?;

            let wallet_seed_hash_array: [u8; 32] =
                wallet_seed.try_into().expect("Seed should be 64 bytes");
            let tx: Transaction = deserialize(&tx_data).expect("Failed to deserialize transaction");

            // Ensure the transaction payload is AssetLockPayloadType
            let Some(TransactionPayload::AssetLockPayloadType(payload)) =
                &tx.special_transaction_payload
            else {
                panic!("Expected AssetLockPayloadType in special_transaction_payload");
            };

            // Get the first credit output
            let first = payload
                .credit_outputs
                .first()
                .expect("Expected at least one credit output");

            let address =
                Address::from_script(&first.script_pubkey, *network).expect("expected an address");

            let (islock, proof) = if let Some(islock_bytes) = islock_data {
                // Deserialize the InstantLock
                let is_lock: InstantLock =
                    deserialize(&islock_bytes).expect("Failed to deserialize InstantLock");
                (
                    Some(is_lock.clone()),
                    Some(AssetLockProof::Instant(InstantAssetLockProof::new(
                        is_lock,
                        tx.clone(),
                        0,
                    ))),
                )
            } else if let Some(chain_locked_height) = chain_locked_height {
                (
                    None,
                    Some(AssetLockProof::Chain(ChainAssetLockProof {
                        core_chain_locked_height: chain_locked_height,
                        out_point: OutPoint::new(tx.txid(), 0),
                    })),
                )
            } else {
                (None, None)
            };

            Ok((wallet_seed_hash_array, tx, address, amount, islock, proof))
        })?;

        // Step 7: Add the asset lock transactions to the corresponding wallets.
        for row in asset_lock_rows {
            let (wallet_seed, tx, address, amount, islock, proof) = row?;

            if let Some(wallet) = wallets_map.get_mut(&wallet_seed) {
                wallet
                    .unused_asset_locks
                    .push((tx, address, amount, islock, proof));
            }
        }

        // Step 8: Retrieve identities for each wallet and add them to the wallets.
        let mut identity_stmt = conn.prepare(
            "SELECT data, wallet, wallet_index FROM identity WHERE network = ? AND wallet IS NOT NULL AND wallet_index IS NOT NULL",
        )?;

        let identity_rows = identity_stmt.query_map([network_str.clone()], |row| {
            let data: Vec<u8> = row.get(0)?;
            let wallet_seed_hash: Vec<u8> = row.get(1)?;
            let wallet_index: u32 = row.get(2)?;

            let wallet_seed_hash_array: [u8; 32] = wallet_seed_hash
                .try_into()
                .expect("Seed hash should be 32 bytes");

            Ok((data, wallet_seed_hash_array, wallet_index))
        })?;

        // Process the identities and add them to the corresponding wallets.
        for row in identity_rows {
            let (identity_data, wallet_seed_hash_array, wallet_index) = row?;

            if let Some(wallet) = wallets_map.get_mut(&wallet_seed_hash_array) {
                let identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&identity_data);

                // Insert the identity into the wallet's identities HashMap with wallet_index as the key
                wallet.identities.insert(wallet_index, identity.identity);
            }
        }

        // Convert the BTreeMap into a Vec of Wallets.
        Ok(wallets_map.into_values().collect())
    }
}
