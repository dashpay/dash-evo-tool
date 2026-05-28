use crate::database::{CorruptedBlobError, Database};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{
    AddressInfo, ClosedKeyItem, DerivationPathReference, DerivationPathType, OpenWalletSeed,
    Wallet, WalletSeed, WalletTransaction,
};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::dashcore::address::{NetworkChecked, NetworkUnchecked};
use dash_sdk::dpp::dashcore::consensus::serialize;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{self, Network};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, ExtendedPubKey};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

impl Database {
    /// Insert a new wallet into the wallet table
    pub fn store_wallet(&self, wallet: &Wallet, network: &Network) -> rusqlite::Result<()> {
        self.store_wallet_with_addresses(wallet, network, &[])
    }

    /// Atomically persist a wallet row and its known addresses in a single
    /// database transaction. Prevents partial persistence where the wallet
    /// is stored but addresses are lost on failure.
    pub fn store_wallet_with_addresses(
        &self,
        wallet: &Wallet,
        network: &Network,
        addresses: &[(
            &Address,
            &DerivationPath,
            DerivationPathReference,
            DerivationPathType,
        )],
    ) -> rusqlite::Result<()> {
        let network_str = network.to_string();

        let master_ecdsa_bip44_account_0_epk_bytes =
            wallet.master_bip44_ecdsa_extended_public_key.encode();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint, network, core_wallet_name)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                network_str,
                wallet.core_wallet_name.as_deref(),
            ],
        )?;

        let seed_hash = wallet.seed_hash();
        for (address, derivation_path, path_reference, path_type) in addresses {
            let checked_addr = check_address_for_network(address.as_unchecked().clone(), network)?;
            tx.execute(
                "INSERT OR IGNORE INTO wallet_addresses
                 (seed_hash, address, derivation_path, path_reference, path_type, balance)
                 VALUES (?, ?, ?, ?, ?, NULL)",
                params![
                    seed_hash,
                    checked_addr.to_string(),
                    derivation_path.to_string(),
                    *path_reference as u32,
                    path_type.bits(),
                ],
            )?;
        }

        tx.commit()
    }

    /// Update the Dash Core wallet name for an HD wallet.
    ///
    /// Returns `Ok(true)` if exactly one row was updated, `Ok(false)` if no
    /// matching wallet was found (0 rows), or `Err` on database errors
    /// (including the unexpected case of >1 rows affected).
    pub fn set_wallet_core_wallet_name(
        &self,
        seed_hash: &[u8; 32],
        core_wallet_name: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE wallet SET core_wallet_name = ? WHERE seed_hash = ?",
            params![core_wallet_name, seed_hash],
        )?;
        match rows {
            0 => Ok(false),
            1 => Ok(true),
            n => Err(rusqlite::Error::StatementChangedRows(n)),
        }
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

    /// Remove a wallet and all associated records from the database.
    ///
    /// This clears dependent records (addresses, utxos, asset locks, identity links)
    /// to keep the database consistent before deleting the wallet itself.
    pub fn remove_wallet(&self, seed_hash: &[u8; 32], network: &Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut address_stmt =
            tx.prepare("SELECT address FROM wallet_addresses WHERE seed_hash = ?")?;
        let address_rows =
            address_stmt.query_map(params![seed_hash], |row| row.get::<_, String>(0))?;
        let mut addresses = Vec::new();
        for address in address_rows {
            addresses.push(address?);
        }
        drop(address_stmt);

        for address in addresses {
            tx.execute(
                "DELETE FROM utxos WHERE address = ? AND network = ?",
                params![address, &network_str],
            )?;
        }

        tx.execute(
            "UPDATE identity SET wallet = NULL, wallet_index = NULL WHERE wallet = ? AND network = ?",
            params![seed_hash, &network_str],
        )?;

        tx.execute(
            "DELETE FROM wallet WHERE seed_hash = ? AND network = ?",
            params![seed_hash, &network_str],
        )?;

        tx.commit()
    }

    /// Update only the alias and is_main fields of a wallet
    #[allow(dead_code)] // May be used for batch wallet metadata updates
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
    #[allow(clippy::too_many_arguments)]
    pub fn add_address_if_not_exists(
        &self,
        seed_hash: &[u8; 32],
        address: &Address,
        network: &Network,
        derivation_path: &DerivationPath,
        path_reference: DerivationPathReference,
        path_type: DerivationPathType,
        balance: Option<u64>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        let address = check_address_for_network(address.as_unchecked().clone(), network)?;

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

    /// Migration: Add balance columns to wallet table (version 16).
    pub fn add_wallet_balance_columns(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if confirmed_balance column exists
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet') WHERE name='confirmed_balance'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN confirmed_balance INTEGER DEFAULT 0;",
                (),
            )?;
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN unconfirmed_balance INTEGER DEFAULT 0;",
                (),
            )?;
            conn.execute(
                "ALTER TABLE wallet ADD COLUMN total_balance INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Migration: Add total_received column to wallet_addresses table.
    pub fn add_address_total_received_column(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if total_received column exists
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('wallet_addresses') WHERE name='total_received'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE wallet_addresses ADD COLUMN total_received INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Ensures all required columns exist in wallet-related tables.
    /// This handles the case where old tables exist with missing columns.
    pub fn ensure_wallet_columns_exist(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if wallet_addresses table exists before trying to add columns
        let wallet_addresses_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wallet_addresses'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if wallet_addresses_exists {
            self.add_address_total_received_column(conn)?;
        }

        // Check if wallet table exists and add balance columns if needed
        let wallet_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wallet'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if wallet_exists {
            self.add_wallet_balance_columns(conn)?;
        }

        Ok(())
    }

    /// Update the total_received for an address.
    pub fn initialize_wallet_transactions_table(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS wallet_transactions (
                seed_hash BLOB NOT NULL,
                txid BLOB NOT NULL,
                network TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                height INTEGER,
                block_hash BLOB,
                net_amount INTEGER NOT NULL,
                fee INTEGER,
                label TEXT,
                is_ours INTEGER NOT NULL,
                raw_transaction BLOB NOT NULL,
                status INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (seed_hash, txid, network),
                FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_wallet_transactions_network_ts
             ON wallet_transactions (network, timestamp DESC)",
            [],
        )?;

        Ok(())
    }

    /// Replace all persisted transactions for a wallet+network with the provided set.
    ///
    /// Uses `INSERT OR REPLACE` so that when upstream returns the same txid
    /// twice (e.g. as mempool + confirmed), the last-written version wins.
    /// Callers should sort confirmed entries after unconfirmed to ensure the
    /// confirmed version takes precedence.
    pub fn replace_wallet_transactions(
        &self,
        seed_hash: &[u8; 32],
        network: &Network,
        transactions: &[WalletTransaction],
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let network_str = network.to_string();

        tx.execute(
            "DELETE FROM wallet_transactions WHERE seed_hash = ?1 AND network = ?2",
            params![seed_hash, &network_str],
        )?;

        if transactions.is_empty() {
            tx.commit()?;
            return Ok(());
        }

        {
            let mut insert_stmt = tx.prepare(
                "INSERT OR REPLACE INTO wallet_transactions (
                    seed_hash,
                    txid,
                    network,
                    timestamp,
                    height,
                    block_hash,
                    net_amount,
                    fee,
                    label,
                    is_ours,
                    raw_transaction,
                    status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;

            for transaction in transactions {
                let tx_bytes = serialize(&transaction.transaction);
                let block_hash_bytes: Option<Vec<u8>> = transaction
                    .block_hash
                    .as_ref()
                    .map(|hash| hash.as_raw_hash().as_byte_array().to_vec());
                let fee = transaction.fee.map(|f| f as i64);
                insert_stmt.execute(params![
                    seed_hash,
                    <dash_sdk::dpp::dashcore::Txid as AsRef<[u8]>>::as_ref(&transaction.txid),
                    &network_str,
                    transaction.timestamp as i64,
                    transaction.height.map(|h| h as i64),
                    block_hash_bytes.as_deref(),
                    transaction.net_amount,
                    fee,
                    transaction.label.as_deref(),
                    transaction.is_ours,
                    tx_bytes,
                    transaction.status as u8,
                ])?;
            }
        }

        tx.commit()
    }

    /// Retrieve all wallets for a specific network, including their addresses, balances, and known addresses.
    ///
    /// Stops on the first corrupted identity blob and returns an error for
    /// the entire call. This is intentional — identities hold private keys
    /// and balance data, so skipping a corrupted entry could cause loss of
    /// funds.
    pub fn get_wallets(&self, network: &Network) -> rusqlite::Result<Vec<Wallet>> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();

        tracing::trace!("step 1: retrieve all wallets for the given network");
        let mut stmt = conn.prepare(
            "SELECT seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, alias, is_main, uses_password, password_hint, core_wallet_name FROM wallet WHERE network = ?",
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
            let core_wallet_name: Option<String> = row.get(9)?;

            // Reconstruct the extended public keys
            let master_ecdsa_extended_public_key =
                ExtendedPubKey::decode(&master_ecdsa_bip44_account_0_epk_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Blob,
                        Box::new(CorruptedBlobError(format!(
                            "Failed to decode ExtendedPubKey: {}",
                            e
                        ))),
                    )
                })?;

            let seed_hash_array: [u8; 32] = seed_hash.try_into().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    Box::new(CorruptedBlobError(
                        "Seed hash should be 32 bytes".to_string(),
                    )),
                )
            })?;
            let closed_wallet_seed = ClosedKeyItem {
                seed_hash: seed_hash_array,
                encrypted_seed: encrypted_seed.clone(),
                salt,
                nonce,
                password_hint,
            };
            let wallet_seed = if uses_password {
                WalletSeed::Closed(closed_wallet_seed)
            } else {
                WalletSeed::Open(OpenWalletSeed {
                    seed: encrypted_seed.try_into().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Blob,
                            Box::new(CorruptedBlobError(
                                "Seed should be 64 bytes for open wallet".to_string(),
                            )),
                        )
                    })?,
                    wallet_info: closed_wallet_seed,
                })
            };

            tracing::trace!(
                alias = ?alias,
                wallet_seed = ?seed_hash_array,
                network = network_str,
                "new wallet loaded from database"
            );

            // Insert a new Wallet into the map
            wallets_map.insert(
                seed_hash_array,
                Wallet {
                    wallet_seed,
                    uses_password,
                    master_bip44_ecdsa_extended_public_key: master_ecdsa_extended_public_key,
                    known_addresses: BTreeMap::new(),
                    watched_addresses: BTreeMap::new(),
                    alias,
                    identities: HashMap::new(),
                    is_main,
                    platform_address_info: BTreeMap::new(),
                    core_wallet_name,
                },
            );

            Ok(())
        })?;

        // Collect any errors during wallet row processing
        for wallet in wallet_rows {
            wallet?;
        }

        tracing::trace!(
            "step 2: retrieve all addresses, balances, and derivation paths associated with the wallets"
        );
        let mut address_stmt = conn.prepare(
            "SELECT seed_hash, address, derivation_path, balance, path_reference, path_type, total_received FROM wallet_addresses WHERE seed_hash IN (SELECT seed_hash FROM wallet WHERE network = ?)",
        )?;

        let address_rows = address_stmt.query_map([network_str.clone()], |row| {
            let seed_hash: Vec<u8> = row.get(0)?;
            let address_str: String = row.get(1)?;
            let derivation_path: String = row.get(2)?;
            let balance: Option<u64> = row.get(3)?;
            let path_reference: u32 = row.get(4)?;
            let path_type: u32 = row.get(5)?;
            let total_received: Option<u64> = row.get(6)?;

            let seed_hash_array: [u8; 32] = seed_hash.try_into().map_err(|_| {
                rusqlite::Error::InvalidParameterName(
                    "Seed hash should be 32 bytes".to_string(),
                )
            })?;

            // Convert u32 to DerivationPathReference safely
            let path_reference =
                DerivationPathReference::try_from(path_reference).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(std::fmt::Error),
                    )
                })?;

            // Parse address - Platform addresses (DIP-17/18) use Bech32m encoding with dash/tdash HRP per DIP-18
            // and need special handling when stored (we store as Core address format internally)
            let address = if path_reference == DerivationPathReference::PlatformPayment {
                // Platform addresses are stored as Core P2PKH format for efficient internal lookup.
                // We use assume_checked() here because:
                // 1. Network validation was already performed at insertion time
                // 2. Platform addresses (bech32m) map to Core P2PKH addresses internally
                // 3. The stored address format doesn't have the same network version byte rules
                Address::from_str(&address_str)
                    .map(|a| a.assume_checked())
                    .map_err(|e| {
                        tracing::error!(address = %address_str, error = ?e, "Failed to parse Platform address");
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(std::fmt::Error),
                        )
                    })?
            } else {
                // Standard Core addresses - validate network
                let address_unchecked =
                    Address::from_str(&address_str).map_err(|e| {
                        rusqlite::Error::InvalidParameterName(format!(
                            "Invalid address format '{}': {}",
                            address_str, e
                        ))
                    })?;
                check_address_for_network(address_unchecked, network)?
            };

            let derivation_path = DerivationPath::from_str(&derivation_path).map_err(|e| {
                rusqlite::Error::InvalidParameterName(format!(
                    "Invalid derivation path '{}': {}",
                    derivation_path, e
                ))
            })?;

            let path_type = DerivationPathType::from_bits_truncate(path_type);

            Ok((
                seed_hash_array,
                address,
                derivation_path,
                balance,
                path_reference,
                path_type,
                total_received,
            ))
        })?;

        tracing::trace!("step 3: add addresses, balances, and known addresses to wallets");
        for row in address_rows {
            if row.is_err() {
                continue;
            }
            let (
                seed_array,
                address,
                derivation_path,
                _balance,
                path_reference,
                path_type,
                _total_received,
            ) = row?;
            if let Some(wallet) = wallets_map.get_mut(&seed_array) {
                // Canonicalize Platform addresses to avoid duplicate representations
                let canonical_address = Wallet::canonical_address(&address, *network);

                // Add the address to the `known_addresses` map.
                wallet
                    .known_addresses
                    .insert(canonical_address.clone(), derivation_path.clone());
                tracing::trace!(
                    address = ?canonical_address,
                    network = address.network().to_string(),
                    expected_network = network.to_string(),
                    "loaded address from database");

                // Add the address to the `watched_addresses` map with AddressInfo.
                let address_info = AddressInfo {
                    address: canonical_address.clone(),
                    path_reference,
                    path_type,
                };
                wallet
                    .watched_addresses
                    .insert(derivation_path, address_info);
            }
        }

        // Step 4: asset-lock state lives in the upstream `AssetLockManager`
        // (queried via `WalletBackend::list_tracked_asset_locks`). The
        // `asset_lock_transaction` SQLite table is preserved as a dormant
        // artifact for a future migration tool but is no longer read here.

        tracing::trace!(
            network = network_str,
            "step 8: retrieve identities for wallets"
        );
        let mut identity_stmt = conn.prepare(
            "SELECT data, wallet, wallet_index FROM identity WHERE network = ? AND wallet IS NOT NULL AND wallet_index IS NOT NULL",
        )?;

        let identity_rows = identity_stmt.query_map([network_str.clone()], |row| {
            let data: Vec<u8> = row.get(0)?;
            let wallet_seed_hash: Vec<u8> = row.get(1)?;
            let wallet_index: u32 = row.get(2)?;

            let wallet_seed_hash_array: [u8; 32] = wallet_seed_hash.try_into().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Blob,
                    Box::new(CorruptedBlobError(
                        "Identity wallet seed hash should be 32 bytes".to_string(),
                    )),
                )
            })?;

            Ok((data, wallet_seed_hash_array, wallet_index))
        })?;
        // Process the identities and add them to the corresponding wallets.
        for row in identity_rows {
            let (identity_data, wallet_seed_hash_array, wallet_index) = row?;

            if let Some(wallet) = wallets_map.get_mut(&wallet_seed_hash_array) {
                let mut identity = QualifiedIdentity::from_bytes(&identity_data).map_err(|e| {
                    tracing::warn!(wallet_index, error = %e, "found corrupted identity blob");
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        CorruptedBlobError(format!(
                            "Failed to deserialize identity for wallet_index {}: {}",
                            wallet_index, e
                        ))
                        .into(),
                    )
                })?;
                identity.wallet_index = Some(wallet_index);
                identity.network = *network;

                tracing::trace!(
                    wallet_seed = hex::encode(wallet_seed_hash_array),
                    wallet_alias = ?wallet.alias,
                    identity = ?identity.identity.id().to_string(Encoding::Base58),
                    identity_alias = ?identity.alias,
                    wallet_index = wallet_index,
                    "adding identity to wallet"
                );
                // Insert the identity into the wallet's identities HashMap with wallet_index as the key
                wallet.identities.insert(wallet_index, identity.identity);
            }
        }

        // Platform address-info + sync cursor live in the per-wallet
        // k/v store; rehydrated by
        // `AppContext::restore_platform_address_info_from_kv` once the
        // wallet backend is wired.

        Ok(wallets_map.into_values().collect())
    }

    /// Clear all Platform receive addresses for a network (developer tool).
    ///
    /// Operates on the `wallet_addresses` table only. The per-wallet
    /// k/v slots holding balance + nonce and the sync cursor are cleared
    /// by the caller (see the network-chooser dev-tool button).
    pub fn clear_all_platform_addresses(&self, network: &Network) -> rusqlite::Result<usize> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();

        // Delete platform addresses from wallet_addresses (path_reference = 16 is PlatformPayment)
        // We need to join with wallet table to filter by network
        let deleted = conn.execute(
            "DELETE FROM wallet_addresses
             WHERE path_reference = 16
             AND seed_hash IN (SELECT seed_hash FROM wallet WHERE network = ?)",
            params![network_str],
        )?;

        Ok(deleted)
    }
}

/// Ensure the address is valid for the given network and
/// update its network if necessary.
///
/// Consumes the address and returns a new Address with the correct network.
fn check_address_for_network(
    address_unchecked: Address<NetworkUnchecked>,
    network: &Network,
) -> Result<Address<NetworkChecked>, WalletError> {
    let address_checked = address_unchecked
        .require_network(*network)
        .inspect_err(|e| {
            tracing::error!("address is not valid for the network: {}", e);
        })?;

    // For devnet/regtest addresses, require_network() accepts testnet addresses; we need to overwrite it here in case there is
    // a mismatch to match the network we are using.
    //
    // See also logic in [`Address::is_valid_for_network()`].
    match address_checked.network() {
        // When the address is correct, do nothing
        address_network if network == address_network => Ok(address_checked),
        // For devnet/regtest addresses, address type can default to testnet, require_network() accepts this;
        //  we need to overwrite it with correct network.
        Network::Testnet if network == &Network::Devnet || network == &Network::Regtest => {
            Ok(Address::new(*network, address_checked.payload().clone()))
        }
        // other cases, like mainnet or testnet, return an error on mismatch
        address_network => {
            tracing::error!(address = ?address_checked,
            network = address_network.to_string(),
            required_network = network.to_string(),
            "address has invalid network set");

            Err(WalletError::AddressError(
                dashcore::address::Error::NetworkValidation {
                    required: *network,
                    found: *address_checked.network(),
                    address: address_checked.as_unchecked().clone(),
                },
            ))
        }
    }
}

#[derive(thiserror::Error, Debug)]
/// Error type for wallet operations.
pub enum WalletError {
    /// Invalid address format.
    #[error("The wallet address could not be read. Please check the format and try again.")]
    AddressError(#[from] dashcore::address::Error),

    /// HD key derivation failed (BIP-32/BIP-44).
    #[error(
        "Could not derive a wallet key. The wallet may be corrupted — try re-importing your recovery phrase."
    )]
    KeyDerivation {
        #[from]
        source: dash_sdk::dpp::key_wallet::bip32::Error,
    },

    /// Signature hash computation failed during transaction signing.
    #[error("Could not prepare the transaction for signing. Please retry.")]
    Sighash {
        /// Zero-based index of the transaction input that failed.
        input_index: usize,
        #[source]
        source: dash_sdk::dpp::dashcore::sighash::Error,
    },
}

impl From<WalletError> for rusqlite::Error {
    fn from(err: WalletError) -> Self {
        rusqlite::Error::UserFunctionError(Box::new(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;
    use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, ExtendedPrivKey};

    fn create_test_seed_hash() -> [u8; 32] {
        let mut hash = [0u8; 32];
        for (i, byte) in hash.iter_mut().enumerate() {
            *byte = i as u8;
        }
        hash
    }

    fn create_test_master_epk_bytes(network: Network) -> Vec<u8> {
        let seed = [7u8; 64];
        let secp = Secp256k1::new();
        let master = ExtendedPrivKey::new_master(network, &seed).expect("master key");
        let path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
        ]);
        let account = master
            .derive_priv(&secp, &path)
            .expect("derive bip44 account");
        ExtendedPubKey::from_priv(&secp, &account).encode()
    }

    #[test]
    fn test_get_wallets_invalid_epk_uses_from_sql_conversion_failure() {
        let db = create_test_database().expect("Failed to create test database");
        let seed_hash = create_test_seed_hash();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk,
                    alias, is_main, uses_password, password_hint, network,
                    confirmed_balance, unconfirmed_balance, total_balance
                 ) VALUES (?, ?, ?, ?, ?, NULL, 0, 0, NULL, 'testnet', 0, 0, 0)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    vec![0u8; 64],
                    vec![0u8; 16],
                    vec![0u8; 12],
                    vec![0u8; 78],
                ],
            )
            .expect("Failed to insert test wallet");
        }

        let err = db
            .get_wallets(&Network::Testnet)
            .expect_err("expected failure");
        match err {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Blob, _) => {}
            _ => panic!("unexpected error variant: {}", err),
        }
    }

    #[test]
    fn test_get_wallets_invalid_seed_hash_length_uses_from_sql_conversion_failure() {
        let db = create_test_database().expect("Failed to create test database");
        let valid_epk = create_test_master_epk_bytes(Network::Testnet);

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk,
                    alias, is_main, uses_password, password_hint, network,
                    confirmed_balance, unconfirmed_balance, total_balance
                 ) VALUES (?, ?, ?, ?, ?, NULL, 0, 0, NULL, 'testnet', 0, 0, 0)",
                rusqlite::params![
                    vec![0u8; 31],
                    vec![0u8; 64],
                    vec![0u8; 16],
                    vec![0u8; 12],
                    valid_epk,
                ],
            )
            .expect("Failed to insert test wallet");
        }

        let err = db
            .get_wallets(&Network::Testnet)
            .expect_err("expected failure");
        match err {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, _) => {}
            _ => panic!("unexpected error variant: {}", err),
        }
    }

    #[test]
    fn test_get_wallets_invalid_open_seed_length_uses_from_sql_conversion_failure() {
        let db = create_test_database().expect("Failed to create test database");
        let seed_hash = create_test_seed_hash();
        let valid_epk = create_test_master_epk_bytes(Network::Testnet);

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO wallet (
                    seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk,
                    alias, is_main, uses_password, password_hint, network,
                    confirmed_balance, unconfirmed_balance, total_balance
                 ) VALUES (?, ?, ?, ?, ?, NULL, 0, 0, NULL, 'testnet', 0, 0, 0)",
                rusqlite::params![
                    seed_hash.as_slice(),
                    vec![0u8; 63],
                    vec![0u8; 16],
                    vec![0u8; 12],
                    valid_epk,
                ],
            )
            .expect("Failed to insert test wallet");
        }

        let err = db
            .get_wallets(&Network::Testnet)
            .expect_err("expected failure");
        match err {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Blob, _) => {}
            _ => panic!("unexpected error variant: {}", err),
        }
    }

    #[test]
    fn test_set_wallet_alias() {
        let db = create_test_database().expect("Failed to create test database");
        let seed_hash = create_test_seed_hash();

        // Insert test wallet first
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO wallet (seed_hash, encrypted_seed, salt, nonce, master_ecdsa_bip44_account_0_epk, uses_password, network)
                 VALUES (?, ?, ?, ?, ?, 0, 'testnet')",
                rusqlite::params![
                    seed_hash.as_slice(),
                    vec![0u8; 64],
                    vec![0u8; 16],
                    vec![0u8; 12],
                    vec![0u8; 78],
                ],
            )
            .expect("Failed to insert test wallet");
        }

        // Set alias
        db.set_wallet_alias(&seed_hash, Some("My Wallet".to_string()))
            .expect("Failed to set wallet alias");

        // Verify
        let conn = db.conn.lock().unwrap();
        let alias: Option<String> = conn
            .query_row(
                "SELECT alias FROM wallet WHERE seed_hash = ?",
                rusqlite::params![seed_hash.as_slice()],
                |row| row.get(0),
            )
            .expect("Failed to query alias");
        assert_eq!(alias, Some("My Wallet".to_string()));

        drop(conn);

        // Clear alias
        db.set_wallet_alias(&seed_hash, None)
            .expect("Failed to clear wallet alias");

        let conn = db.conn.lock().unwrap();
        let alias: Option<String> = conn
            .query_row(
                "SELECT alias FROM wallet WHERE seed_hash = ?",
                rusqlite::params![seed_hash.as_slice()],
                |row| row.get(0),
            )
            .expect("Failed to query alias");
        assert!(alias.is_none());
    }
}
