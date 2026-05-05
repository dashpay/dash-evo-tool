use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::database::Database;
use crate::model::qualified_contract::{QualifiedContract, QualifiedContractRef};
use crate::model::wallet::WalletSeedHash;
use crate::ui::tokens::tokens_screen::{IdentityTokenBalance, IdentityTokenIdentifier};
use bincode::config;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::query_types::IndexMap;
use rusqlite::Result;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Pagination instructions for [`AppContext::get_contracts`].
///
/// `system_skip` and `system_take` apply to the in-memory list of cached system
/// contracts; `db_offset` and `db_limit` are then used to fetch additional
/// rows from the database starting at the system-contract boundary.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContractPagination {
    pub system_skip: usize,
    pub system_take: usize,
    pub db_offset: u32,
    pub db_limit: Option<u32>,
}

/// Splits a `(limit, offset)` request across the cached system contracts and
/// the database-backed contracts.
///
/// The system contracts are conceptually the first `system_contract_count`
/// entries of the combined contract list. This helper computes how many of
/// those system entries to skip/take, and the residual `(offset, limit)` to
/// hand to the database query.
pub(crate) fn compute_contract_pagination(
    system_contract_count: u32,
    limit: Option<u32>,
    offset: Option<u32>,
) -> ContractPagination {
    let effective_offset = offset.unwrap_or(0);
    let visible_system_contracts = system_contract_count.saturating_sub(effective_offset);
    let system_take = limit
        .map(|limit| visible_system_contracts.min(limit))
        .unwrap_or(visible_system_contracts);
    let db_offset = effective_offset.saturating_sub(system_contract_count);
    let db_limit = limit.map(|limit| limit.saturating_sub(system_take));

    ContractPagination {
        system_skip: effective_offset.min(system_contract_count) as usize,
        system_take: system_take as usize,
        db_offset,
        db_limit,
    }
}

/// Removes any persisted rows whose contract IDs are listed as system
/// contracts. Used to recover from earlier code paths that incorrectly
/// duplicated system contracts into the local database.
///
/// Performs the cleanup atomically in a single transaction — either every
/// listed system contract (and its dependent token / balance / order rows)
/// is removed or no rows are touched.
pub(crate) fn cleanup_persisted_system_contracts_for(
    db: &Database,
    system_contract_ids: &[Identifier],
    network: &Network,
) -> Result<()> {
    if system_contract_ids.is_empty() {
        return Ok(());
    }

    let network_str = network.to_string();
    let placeholders = vec!["?"; system_contract_ids.len()].join(", ");

    let conn = db.shared_connection();
    let conn = conn.lock().unwrap();
    let tx = conn.unchecked_transaction()?;

    // identity_token_balances → tokens belonging to the listed contracts on this network.
    let sql = format!(
        "DELETE FROM identity_token_balances
         WHERE network = ?
           AND token_id IN (
               SELECT id FROM token
               WHERE network = ? AND data_contract_id IN ({placeholders})
           )"
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(2 + system_contract_ids.len());
    params_vec.push(&network_str);
    params_vec.push(&network_str);
    let id_byte_vecs: Vec<Vec<u8>> = system_contract_ids.iter().map(|id| id.to_vec()).collect();
    for id_bytes in &id_byte_vecs {
        params_vec.push(id_bytes);
    }
    tx.execute(&sql, rusqlite::params_from_iter(params_vec))?;

    // token_order has no `network` column; scope by joining against `token`.
    let sql = format!(
        "DELETE FROM token_order
         WHERE token_id IN (
             SELECT id FROM token
             WHERE network = ? AND data_contract_id IN ({placeholders})
         )"
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(1 + system_contract_ids.len());
    params_vec.push(&network_str);
    for id_bytes in &id_byte_vecs {
        params_vec.push(id_bytes);
    }
    tx.execute(&sql, rusqlite::params_from_iter(params_vec))?;

    // tokens for the listed contracts on this network.
    let sql = format!(
        "DELETE FROM token
         WHERE network = ? AND data_contract_id IN ({placeholders})"
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(1 + system_contract_ids.len());
    params_vec.push(&network_str);
    for id_bytes in &id_byte_vecs {
        params_vec.push(id_bytes);
    }
    tx.execute(&sql, rusqlite::params_from_iter(params_vec))?;

    // contract rows themselves.
    let sql = format!(
        "DELETE FROM contract
         WHERE network = ? AND contract_id IN ({placeholders})"
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(1 + system_contract_ids.len());
    params_vec.push(&network_str);
    for id_bytes in &id_byte_vecs {
        params_vec.push(id_bytes);
    }
    tx.execute(&sql, rusqlite::params_from_iter(params_vec))?;

    tx.commit()
}

impl AppContext {
    /// Returns borrowed references to cached system-contract `Arc`s with aliases.
    ///
    /// Use this for read-only lookups (ID checks, membership tests, `Arc` cloning)
    /// to avoid deep-cloning cached `DataContract`s.
    fn system_contract_arcs(&self) -> [(&Arc<DataContract>, &'static str); 5] {
        [
            (&self.dpns_contract, "dpns"),
            (&self.token_history_contract, "token_history"),
            (&self.withdraws_contract, "withdrawals"),
            (&self.keyword_search_contract, "keyword_search"),
            (&self.dashpay_contract, "dashpay"),
        ]
    }

    pub(crate) fn system_contract_ids(&self) -> [Identifier; 5] {
        self.system_contract_arcs()
            .map(|(system_contract, _)| system_contract.id())
    }

    pub(crate) fn system_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Option<QualifiedContract> {
        self.system_contract_arcs()
            .into_iter()
            .find(|(system_contract, _)| system_contract.id() == *contract_id)
            .map(|(system_contract, alias)| QualifiedContract {
                contract: system_contract.as_ref().clone(),
                alias: Some(alias.to_string()),
            })
    }

    /// Returns an `Arc::clone` of a cached system contract by ID, without deep cloning.
    pub(crate) fn system_contract_arc_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Option<Arc<DataContract>> {
        self.system_contract_arcs()
            .into_iter()
            .find(|(system_contract, _)| system_contract.id() == *contract_id)
            .map(|(system_contract, _)| Arc::clone(system_contract))
    }

    pub(crate) fn is_system_contract_id(&self, contract_id: &Identifier) -> bool {
        self.system_contract_arcs()
            .iter()
            .any(|(system_contract, _)| system_contract.id() == *contract_id)
    }

    pub(crate) fn cleanup_persisted_system_contracts(&self) -> Result<()> {
        cleanup_persisted_system_contracts_for(&self.db, &self.system_contract_ids(), &self.network)
    }

    /// Retrieves all contracts from the database plus the system contracts from app context.
    ///
    /// This eagerly clones every cached system `DataContract` and so should be
    /// avoided on per-frame UI paths. Prefer [`Self::get_contracts_arc`] for
    /// read-only listing, [`Self::loaded_contract_ids`] for membership checks,
    /// or [`Self::get_user_contracts`] for callers that explicitly exclude the
    /// built-in contracts.
    pub fn get_contracts(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<QualifiedContract>> {
        Ok(self
            .get_contracts_arc(limit, offset)?
            .into_iter()
            .map(|qc| qc.to_owned_qualified())
            .collect())
    }

    /// Arc-backed listing of system + database contracts.
    ///
    /// Cached system contracts are shared via `Arc::clone` rather than
    /// deep-cloned, which keeps per-frame UI paths cheap. Database-backed
    /// contracts are still deserialized fresh and wrapped in `Arc::new`.
    pub fn get_contracts_arc(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<QualifiedContractRef>> {
        let system_contract_count = self.system_contract_arcs().len() as u32;
        let pagination = compute_contract_pagination(system_contract_count, limit, offset);

        let mut contracts = self
            .system_contract_arcs()
            .into_iter()
            .skip(pagination.system_skip)
            .take(pagination.system_take)
            .map(|(system_contract, alias)| QualifiedContractRef {
                contract: Arc::clone(system_contract),
                alias: Some(alias.to_string()),
            })
            .collect::<Vec<_>>();

        if pagination.db_limit != Some(0) {
            contracts.extend(
                self.db
                    .get_contracts(self, pagination.db_limit, Some(pagination.db_offset))?
                    .into_iter()
                    .map(|qc| QualifiedContractRef {
                        contract: Arc::new(qc.contract),
                        alias: qc.alias,
                    }),
            );
        }

        Ok(contracts)
    }

    /// Returns the IDs of every loaded contract (system + database).
    ///
    /// Database-backed IDs are validated through the same deserialization
    /// path as [`Self::get_contracts`] and [`Self::get_contract_by_id`], so
    /// the returned set matches the visible loaded contract set (rows whose
    /// contract blobs fail to deserialize are skipped just as they are in
    /// the listing helpers). This is **not** a cheap raw-id-only check: its
    /// cost is equivalent to scanning and deserializing every persisted
    /// contract blob, minus the cost of constructing full listing objects.
    pub fn loaded_contract_ids(&self) -> Result<Vec<Identifier>> {
        let mut ids: Vec<Identifier> = self.system_contract_ids().to_vec();
        ids.extend(self.db.get_contract_ids_for_network(self)?);
        Ok(ids)
    }

    /// Returns only the user-added (database-backed) contracts, skipping the
    /// built-in system contracts entirely. Callers that immediately filter
    /// out system contracts should use this rather than building them only to
    /// discard them.
    pub fn get_user_contracts(&self) -> Result<Vec<QualifiedContract>> {
        self.db.get_contracts(self, None, None)
    }

    pub fn get_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Result<Option<QualifiedContract>> {
        if let Some(system_contract) = self.system_contract_by_id(contract_id) {
            return Ok(Some(system_contract));
        }

        // Get the contract from the database
        self.db.get_contract_by_id(*contract_id, self)
    }

    pub fn get_unqualified_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Result<Option<DataContract>> {
        if let Some(system_contract) = self.system_contract_by_id(contract_id) {
            return Ok(Some(system_contract.contract));
        }

        // Get the contract from the database
        self.db.get_unqualified_contract_by_id(*contract_id, self)
    }

    // Remove contract from the database by ID
    pub fn remove_contract(&self, contract_id: &Identifier) -> std::result::Result<(), TaskError> {
        if self.is_system_contract_id(contract_id) {
            tracing::warn!(?contract_id, "Rejecting request to remove system contract");
            return Err(TaskError::SystemContractImmutable {
                contract_id: *contract_id,
            });
        }

        self.db.remove_contract(contract_id.as_bytes(), self)?;
        Ok(())
    }

    pub fn replace_contract(
        &self,
        contract_id: Identifier,
        new_contract: &DataContract,
    ) -> std::result::Result<(), TaskError> {
        if self.is_system_contract_id(&contract_id) {
            tracing::warn!(?contract_id, "Rejecting request to replace system contract");
            return Err(TaskError::SystemContractImmutable { contract_id });
        }

        self.db.replace_contract(contract_id, new_contract, self)?;
        Ok(())
    }

    pub fn identity_token_balances(
        &self,
    ) -> Result<IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>> {
        self.db.get_identity_token_balances(self)
    }

    pub fn remove_token_balance(
        &self,
        token_id: Identifier,
        identity_id: Identifier,
    ) -> Result<()> {
        self.db.remove_token_balance(&token_id, &identity_id, self)
    }

    pub fn insert_token(
        &self,
        token_id: &Identifier,
        token_name: &str,
        token_configuration: TokenConfiguration,
        contract_id: &Identifier,
        token_position: u16,
    ) -> Result<()> {
        let config = config::standard();
        let Some(serialized_token_configuration) =
            bincode::encode_to_vec(&token_configuration, config).ok()
        else {
            // We should always be able to serialize
            return Ok(());
        };

        self.db.insert_token(
            token_id,
            token_name,
            serialized_token_configuration.as_slice(),
            contract_id,
            token_position,
            self,
        )?;

        Ok(())
    }

    pub fn remove_token(&self, token_id: &Identifier) -> Result<()> {
        self.db.remove_token(token_id, self)
    }

    pub fn remove_wallet(&self, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        // Acquire write lock first to ensure atomicity — if the lock fails,
        // no changes have been made to the database.
        let mut wallets = self.wallets.write()?;
        if !wallets.contains_key(seed_hash) {
            return Err(TaskError::WalletNotFound);
        }

        self.db.remove_wallet(seed_hash, &self.network)?;

        wallets.remove(seed_hash);
        let has_wallet = !wallets.is_empty();
        drop(wallets);

        self.has_wallet.store(has_wallet, Ordering::Relaxed);

        Ok(())
    }

    #[allow(dead_code)] // May be used for storing token balances
    pub fn insert_token_identity_balance(
        &self,
        token_id: &Identifier,
        identity_id: &Identifier,
        balance: u64,
    ) -> Result<()> {
        self.db
            .insert_identity_token_balance(token_id, identity_id, balance, self)?;

        Ok(())
    }

    pub fn get_contract_by_token_id(
        &self,
        token_id: &Identifier,
    ) -> Result<Option<QualifiedContract>> {
        let Some(contract_id) = self.db.get_contract_id_by_token_id(token_id, self)? else {
            return Ok(None);
        };
        self.get_contract_by_id(&contract_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_dir::ensure_env_file;
    use crate::database::test_helpers::create_test_database;
    use dash_sdk::dpp::dashcore::Network;
    use std::sync::Once;

    /// Builds a deterministic 32-byte `Identifier` from a single seed byte.
    fn identifier_with_byte(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32 bytes is a valid identifier")
    }

    /// Sets the minimum env vars required by `Config::load_from` so that
    /// `AppContext::new` can construct a `Network::Regtest` context without
    /// touching the user's real configuration. Mirrors the helper used by
    /// the tokens-screen tests.
    fn ensure_test_env() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            // Safety: tests set env vars once to ensure deterministic config;
            // no other test mutates these specific values.
            unsafe {
                std::env::set_var("MAINNET_dapi_addresses", "http://127.0.0.1:1443");
                std::env::set_var("MAINNET_core_host", "127.0.0.1");
                std::env::set_var("MAINNET_core_rpc_port", "9998");
                std::env::set_var("MAINNET_core_rpc_user", "dashrpc");
                std::env::set_var("MAINNET_core_rpc_password", "password");

                std::env::set_var("LOCAL_dapi_addresses", "http://127.0.0.1:2443");
                std::env::set_var("LOCAL_core_host", "127.0.0.1");
                std::env::set_var("LOCAL_core_rpc_port", "20302");
                std::env::set_var("LOCAL_core_rpc_user", "dashmate");
                std::env::set_var("LOCAL_core_rpc_password", "password");
            }
        });
    }

    fn contract_row_present(db: &Database, id: &Identifier, network: &Network) -> bool {
        let conn = db.shared_connection();
        let conn = conn.lock().unwrap();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM contract WHERE contract_id = ? AND network = ?)",
            rusqlite::params![id.to_vec(), network.to_string()],
            |r| r.get::<_, bool>(0),
        )
        .expect("query exists")
    }

    fn contract_row_bytes(db: &Database, id: &Identifier, network: &Network) -> Option<Vec<u8>> {
        let conn = db.shared_connection();
        let conn = conn.lock().unwrap();
        match conn.query_row(
            "SELECT contract FROM contract WHERE contract_id = ? AND network = ?",
            rusqlite::params![id.to_vec(), network.to_string()],
            |r| r.get::<_, Vec<u8>>(0),
        ) {
            Ok(bytes) => Some(bytes),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => panic!("read contract bytes: {e}"),
        }
    }

    /// Placeholder bytes that a serialized `DataContract` cannot equal —
    /// makes the "no overwrite" assertion unambiguous.
    const PLACEHOLDER: &[u8] = b"placeholder-not-a-real-contract";

    /// Inserts a row whose `contract` blob is intentionally **not** a valid
    /// serialized `DataContract`. Use this only for tests that need to
    /// observe the existence/absence of a row keyed by id (no-overwrite,
    /// cleanup, network scoping). Listings that go through
    /// `Database::get_contracts` will silently drop these rows because
    /// `DataContract::versioned_deserialize` returns an error and the
    /// loop `continue`s — for those tests use [`insert_real_contract_row`].
    fn insert_contract_row(db: &Database, id: &Identifier, network: &Network) {
        db.execute(
            "INSERT INTO contract (contract_id, contract, alias, network) VALUES (?, ?, ?, ?)",
            rusqlite::params![
                id.to_vec(),
                PLACEHOLDER.to_vec(),
                Option::<String>::None,
                network.to_string()
            ],
        )
        .expect("insert contract row");
    }

    /// Inserts a row containing real, round-trippable `DataContract` bytes
    /// keyed by `id`. Clones the cached DPNS system contract, re-stamps it
    /// with the requested id via the DPP setter trait, and serializes
    /// through the platform-version API so `Database::get_contracts`
    /// successfully deserializes it and includes it in pagination results.
    fn insert_real_contract_row(
        app_context: &AppContext,
        db: &Database,
        id: Identifier,
        alias: Option<&str>,
    ) {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Setters;
        use dash_sdk::dpp::serialization::PlatformSerializableWithPlatformVersion;

        let mut contract = (*app_context.dpns_contract).clone();
        contract.set_id(id);
        let bytes = contract
            .serialize_to_bytes_with_platform_version(app_context.platform_version())
            .expect("serialize cloned data contract");

        db.execute(
            "INSERT INTO contract (contract_id, contract, alias, network) VALUES (?, ?, ?, ?)",
            rusqlite::params![id.to_vec(), bytes, alias, app_context.network.to_string()],
        )
        .expect("insert real contract row");
    }

    // ---------- pagination -----------------------------------------------

    /// `system_count` matches what `AppContext::get_contracts` passes today.
    const SYSTEM_COUNT: u32 = 5;

    #[test]
    fn pagination_returns_only_system_when_limit_within_system_range() {
        let p = compute_contract_pagination(SYSTEM_COUNT, Some(2), None);
        assert_eq!(
            p,
            ContractPagination {
                system_skip: 0,
                system_take: 2,
                db_offset: 0,
                db_limit: Some(0), // signals "skip the DB query entirely"
            }
        );
    }

    #[test]
    fn pagination_returns_all_system_when_limit_equals_system_count() {
        let p = compute_contract_pagination(SYSTEM_COUNT, Some(SYSTEM_COUNT), None);
        assert_eq!(
            p,
            ContractPagination {
                system_skip: 0,
                system_take: SYSTEM_COUNT as usize,
                db_offset: 0,
                db_limit: Some(0),
            }
        );
    }

    #[test]
    fn pagination_crosses_boundary_into_db_contracts() {
        // 5 system contracts + first 2 DB rows.
        let p = compute_contract_pagination(SYSTEM_COUNT, Some(7), None);
        assert_eq!(
            p,
            ContractPagination {
                system_skip: 0,
                system_take: 5,
                db_offset: 0,
                db_limit: Some(2),
            }
        );
    }

    #[test]
    fn pagination_skips_some_system_then_crosses_into_db() {
        // offset=3, limit=5: take system[3..5] (2), then 3 DB rows from offset 0.
        let p = compute_contract_pagination(SYSTEM_COUNT, Some(5), Some(3));
        assert_eq!(
            p,
            ContractPagination {
                system_skip: 3,
                system_take: 2,
                db_offset: 0,
                db_limit: Some(3),
            }
        );
    }

    #[test]
    fn pagination_offset_at_boundary_returns_db_only() {
        // offset == system_count: skip all system, query DB from offset 0.
        let p = compute_contract_pagination(SYSTEM_COUNT, Some(3), Some(SYSTEM_COUNT));
        assert_eq!(
            p,
            ContractPagination {
                system_skip: SYSTEM_COUNT as usize,
                system_take: 0,
                db_offset: 0,
                db_limit: Some(3),
            }
        );
    }

    #[test]
    fn pagination_offset_past_boundary_returns_db_only_with_residual_offset() {
        // offset=7: skip all 5 system contracts, DB starts at offset 2.
        let p = compute_contract_pagination(SYSTEM_COUNT, Some(2), Some(7));
        assert_eq!(
            p,
            ContractPagination {
                system_skip: SYSTEM_COUNT as usize,
                system_take: 0,
                db_offset: 2,
                db_limit: Some(2),
            }
        );
    }

    #[test]
    fn pagination_no_limit_with_offset_into_system_returns_remaining_system_and_all_db() {
        // offset=3, no limit: take system[3..5] (2), then all DB rows from offset 0.
        let p = compute_contract_pagination(SYSTEM_COUNT, None, Some(3));
        assert_eq!(
            p,
            ContractPagination {
                system_skip: 3,
                system_take: 2,
                db_offset: 0,
                db_limit: None,
            }
        );
    }

    #[test]
    fn pagination_no_limit_with_offset_past_system_returns_db_only_with_residual_offset() {
        // offset=7, no limit: skip all system, DB at offset 2 with no limit.
        let p = compute_contract_pagination(SYSTEM_COUNT, None, Some(7));
        assert_eq!(
            p,
            ContractPagination {
                system_skip: SYSTEM_COUNT as usize,
                system_take: 0,
                db_offset: 2,
                db_limit: None,
            }
        );
    }

    #[test]
    fn pagination_no_limit_no_offset_returns_all_system_and_all_db() {
        let p = compute_contract_pagination(SYSTEM_COUNT, None, None);
        assert_eq!(
            p,
            ContractPagination {
                system_skip: 0,
                system_take: SYSTEM_COUNT as usize,
                db_offset: 0,
                db_limit: None,
            }
        );
    }

    // ---------- replace_contract / cleanup --------------------------------

    /// `AppContext::replace_contract` must reject any ID listed in the system
    /// contract set — even if a stale row for that ID somehow exists in the
    /// database, calling `replace_contract` with the live system contract
    /// must NOT overwrite the persisted bytes, and the call must surface a
    /// `SystemContractImmutable` error so callers cannot misreport success.
    #[test]
    fn replace_contract_rejects_system_contract_ids() {
        ensure_test_env();
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        ensure_env_file(temp_dir.path());

        let db = Arc::new(create_test_database().expect("create in-memory db"));

        let app_context = AppContext::new(
            temp_dir.path().to_path_buf(),
            Network::Regtest,
            Arc::clone(&db),
            None,
            Default::default(),
            Default::default(),
            eframe::egui::Context::default(),
        )
        .expect("construct AppContext for test");

        // Pick a real cached system contract — DPNS — and use its id.
        // Construction has already run `cleanup_persisted_system_contracts`,
        // so the table is clean for this id.
        let system_id = app_context.dpns_contract.id();
        let system_contract = (*app_context.dpns_contract).clone();
        assert!(
            !contract_row_present(&db, &system_id, &app_context.network),
            "startup cleanup should have left no row for the system contract id"
        );

        // Insert a placeholder row under the system contract id. This is
        // the state we need to be defensive about: a leftover row shadowing
        // a system contract.
        insert_contract_row(&db, &system_id, &app_context.network);
        assert_eq!(
            contract_row_bytes(&db, &system_id, &app_context.network).as_deref(),
            Some(PLACEHOLDER),
        );

        // Now invoke `replace_contract` with the real system contract.
        // The guard should reject this with SystemContractImmutable.
        let err = app_context
            .replace_contract(system_id, &system_contract)
            .expect_err("replace_contract must reject system ids");
        assert!(
            matches!(err, TaskError::SystemContractImmutable { .. }),
            "replace_contract must surface SystemContractImmutable, got {err:?}"
        );

        // The persisted bytes must still be the placeholder — proving the
        // guard short-circuited before any DB write occurred.
        assert_eq!(
            contract_row_bytes(&db, &system_id, &app_context.network).as_deref(),
            Some(PLACEHOLDER),
            "replace_contract must not overwrite a persisted row for a system contract id"
        );
    }

    /// `AppContext::remove_contract` must reject any ID listed in the system
    /// contract set. The dedicated cleanup helper
    /// (`cleanup_persisted_system_contracts_for`) is the only path that
    /// should delete persisted rows for a system contract id; user-driven
    /// `remove_contract` calls must not, and must surface a
    /// `SystemContractImmutable` error rather than silently succeeding.
    #[test]
    fn remove_contract_rejects_system_contract_ids() {
        ensure_test_env();
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        ensure_env_file(temp_dir.path());

        let db = Arc::new(create_test_database().expect("create in-memory db"));

        let app_context = AppContext::new(
            temp_dir.path().to_path_buf(),
            Network::Regtest,
            Arc::clone(&db),
            None,
            Default::default(),
            Default::default(),
            eframe::egui::Context::default(),
        )
        .expect("construct AppContext for test");

        let system_id = app_context.dpns_contract.id();
        assert!(
            !contract_row_present(&db, &system_id, &app_context.network),
            "startup cleanup should have left no row for the system contract id"
        );

        // Insert a placeholder row under the system contract id. A
        // user-driven remove_contract call must not delete it — only the
        // dedicated cleanup helper is authorised to touch system rows.
        insert_contract_row(&db, &system_id, &app_context.network);
        assert!(contract_row_present(&db, &system_id, &app_context.network));

        let err = app_context
            .remove_contract(&system_id)
            .expect_err("remove_contract must reject system ids");
        assert!(
            matches!(err, TaskError::SystemContractImmutable { .. }),
            "remove_contract must surface SystemContractImmutable, got {err:?}"
        );

        assert!(
            contract_row_present(&db, &system_id, &app_context.network),
            "remove_contract must not delete a persisted row for a system contract id"
        );
    }

    #[test]
    fn cleanup_persisted_system_contracts_removes_only_listed_ids() {
        let db = create_test_database().expect("create in-memory db");
        let network = Network::Testnet;

        let system_id = identifier_with_byte(0xAA);
        let unrelated_id = identifier_with_byte(0xBB);

        // Pre-populate both rows; cleanup should target only the system ID.
        insert_contract_row(&db, &system_id, &network);
        insert_contract_row(&db, &unrelated_id, &network);
        assert!(contract_row_present(&db, &system_id, &network));
        assert!(contract_row_present(&db, &unrelated_id, &network));

        cleanup_persisted_system_contracts_for(&db, &[system_id], &network)
            .expect("cleanup succeeds");

        assert!(
            !contract_row_present(&db, &system_id, &network),
            "system contract row should be removed"
        );
        assert!(
            contract_row_present(&db, &unrelated_id, &network),
            "unrelated contract row should remain"
        );
    }

    #[test]
    fn cleanup_persisted_system_contracts_is_idempotent() {
        let db = create_test_database().expect("create in-memory db");
        let network = Network::Testnet;
        let system_id = identifier_with_byte(0x42);

        // No rows present — cleanup must succeed silently.
        cleanup_persisted_system_contracts_for(&db, &[system_id], &network)
            .expect("cleanup with no rows succeeds");
        assert!(!contract_row_present(&db, &system_id, &network));

        // Insert and remove twice; second run is a no-op.
        insert_contract_row(&db, &system_id, &network);
        cleanup_persisted_system_contracts_for(&db, &[system_id], &network).unwrap();
        cleanup_persisted_system_contracts_for(&db, &[system_id], &network).unwrap();
        assert!(!contract_row_present(&db, &system_id, &network));
    }

    #[test]
    fn cleanup_persisted_system_contracts_is_network_scoped() {
        let db = create_test_database().expect("create in-memory db");
        let id = identifier_with_byte(0x77);

        // Same contract id persisted under two networks; cleanup for testnet
        // must leave the mainnet row untouched.
        insert_contract_row(&db, &id, &Network::Testnet);
        insert_contract_row(&db, &id, &Network::Mainnet);

        cleanup_persisted_system_contracts_for(&db, &[id], &Network::Testnet).unwrap();

        assert!(!contract_row_present(&db, &id, &Network::Testnet));
        assert!(contract_row_present(&db, &id, &Network::Mainnet));
    }

    // ---------- arc-backed listing helpers --------------------------------

    fn make_test_app_context(temp_dir: &std::path::Path, db: Arc<Database>) -> Arc<AppContext> {
        ensure_test_env();
        ensure_env_file(temp_dir);
        AppContext::new(
            temp_dir.to_path_buf(),
            Network::Regtest,
            db,
            None,
            Default::default(),
            Default::default(),
            eframe::egui::Context::default(),
        )
        .expect("construct AppContext for test")
    }

    #[test]
    fn loaded_contract_ids_returns_system_ids_when_db_empty() {
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        let db = Arc::new(create_test_database().expect("create in-memory db"));
        let app_context = make_test_app_context(temp_dir.path(), Arc::clone(&db));

        let ids = app_context.loaded_contract_ids().expect("loaded ids");
        let mut expected: Vec<Identifier> = app_context.system_contract_ids().to_vec();
        let mut got = ids.clone();
        expected.sort();
        got.sort();
        assert_eq!(got, expected, "expected exactly the system contract ids");
    }

    #[test]
    fn loaded_contract_ids_includes_user_contracts() {
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        let db = Arc::new(create_test_database().expect("create in-memory db"));
        let app_context = make_test_app_context(temp_dir.path(), Arc::clone(&db));

        let user_id = identifier_with_byte(0xC0);
        // Use real serialized contract bytes — `loaded_contract_ids` skips
        // rows whose blob fails to deserialize (consistency with
        // `get_contracts` / `get_contract_by_id`).
        insert_real_contract_row(&app_context, &db, user_id, None);

        let ids = app_context.loaded_contract_ids().expect("loaded ids");
        assert!(
            ids.contains(&user_id),
            "user contract id should appear in loaded_contract_ids"
        );
        for system_id in app_context.system_contract_ids() {
            assert!(
                ids.contains(&system_id),
                "system contract id {system_id:?} should appear in loaded_contract_ids"
            );
        }
    }

    /// A row with a valid 32-byte contract id but a contract blob that
    /// `DataContract::versioned_deserialize` cannot parse must NOT show up
    /// in `loaded_contract_ids`. Otherwise `add_contracts` would refuse to
    /// add a contract under that id ("already loaded") even though it is
    /// invisible to every other listing path (`get_contracts`,
    /// `get_contract_by_id`, etc.).
    #[test]
    fn loaded_contract_ids_skips_rows_with_malformed_contract_blob() {
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        let db = Arc::new(create_test_database().expect("create in-memory db"));
        let app_context = make_test_app_context(temp_dir.path(), Arc::clone(&db));

        // Two rows on the active network: one with real serialized bytes,
        // one with a non-deserializable placeholder. Both have valid
        // 32-byte ids — only the body is malformed.
        let healthy_id = identifier_with_byte(0xD0);
        let malformed_id = identifier_with_byte(0xD1);
        insert_real_contract_row(&app_context, &db, healthy_id, None);
        insert_contract_row(&db, &malformed_id, &app_context.network);

        let ids = app_context.loaded_contract_ids().expect("loaded ids");
        assert!(
            ids.contains(&healthy_id),
            "healthy contract id should still appear in loaded_contract_ids",
        );
        assert!(
            !ids.contains(&malformed_id),
            "row with valid id but malformed blob must not appear in loaded_contract_ids",
        );

        // Sanity: the malformed row is also invisible to the listing path —
        // proving `loaded_contract_ids` and `get_contracts` agree on the
        // visible set.
        let listed: std::collections::HashSet<Identifier> = app_context
            .get_contracts(None, None)
            .expect("get_contracts")
            .into_iter()
            .map(|qc| qc.contract.id())
            .collect();
        assert!(
            !listed.contains(&malformed_id),
            "row with malformed blob must not appear in get_contracts either",
        );
    }

    #[test]
    fn get_user_contracts_excludes_system_contracts() {
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        let db = Arc::new(create_test_database().expect("create in-memory db"));
        let app_context = make_test_app_context(temp_dir.path(), Arc::clone(&db));

        // Insert a row whose id collides with a system contract using *real*
        // serialized DataContract bytes. Without real bytes the test would be
        // tautologically true: `Database::get_contracts` skips rows whose
        // contract blob fails to deserialize, so a placeholder row would be
        // dropped by deserialization rather than by the SQL `NOT IN`
        // exclusion we want to verify.
        let system_id = app_context.dpns_contract.id();
        insert_real_contract_row(&app_context, &db, system_id, Some("shadow"));

        // Seed two sibling user contracts that should round-trip through the
        // listing — these prove the listing is still healthy.
        let sibling_a = identifier_with_byte(0xA0);
        let sibling_b = identifier_with_byte(0xA1);
        insert_real_contract_row(&app_context, &db, sibling_a, Some("a"));
        insert_real_contract_row(&app_context, &db, sibling_b, Some("b"));

        // get_user_contracts must omit the system-id row but include the
        // sibling user rows.
        let user_contracts = app_context
            .get_user_contracts()
            .expect("get_user_contracts succeeds");
        let user_ids: std::collections::HashSet<Identifier> =
            user_contracts.iter().map(|qc| qc.contract.id()).collect();
        assert!(
            !user_ids.contains(&system_id),
            "get_user_contracts must exclude rows persisted under a system contract id",
        );
        assert!(
            user_ids.contains(&sibling_a),
            "get_user_contracts must return sibling user contract a",
        );
        assert!(
            user_ids.contains(&sibling_b),
            "get_user_contracts must return sibling user contract b",
        );
        for qc in &user_contracts {
            assert!(
                !app_context.is_system_contract_id(&qc.contract.id()),
                "get_user_contracts must not return any system contract"
            );
        }

        // get_contracts(None, None) must also exclude the persisted system-id
        // row (no duplicate of the cached system contract) while still
        // returning the sibling user rows alongside the cached system list.
        let full = app_context
            .get_contracts(None, None)
            .expect("get_contracts succeeds");
        let mut system_id_count = 0usize;
        for qc in &full {
            if qc.contract.id() == system_id {
                system_id_count += 1;
            }
        }
        assert_eq!(
            system_id_count, 1,
            "system contract id must appear exactly once (cached system entry, not persisted row)",
        );
        let full_ids: std::collections::HashSet<Identifier> =
            full.iter().map(|qc| qc.contract.id()).collect();
        assert!(
            full_ids.contains(&sibling_a),
            "get_contracts must include sibling user contract a",
        );
        assert!(
            full_ids.contains(&sibling_b),
            "get_contracts must include sibling user contract b",
        );
    }

    #[test]
    fn get_contracts_arc_shares_system_contract_allocations() {
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        let db = Arc::new(create_test_database().expect("create in-memory db"));
        let app_context = make_test_app_context(temp_dir.path(), Arc::clone(&db));

        let listed = app_context
            .get_contracts_arc(None, None)
            .expect("get_contracts_arc succeeds");

        // The DPNS contract Arc returned by the listing must point at the
        // same allocation as the one cached on AppContext — that is the
        // whole point of the helper (no deep clone).
        let dpns_listed = listed
            .iter()
            .find(|qc| qc.contract.id() == app_context.dpns_contract.id())
            .expect("listing includes dpns");
        assert!(
            Arc::ptr_eq(&dpns_listed.contract, &app_context.dpns_contract),
            "system contract Arc should be shared with the cached field"
        );
    }

    #[test]
    fn get_contracts_arc_pagination_matches_get_contracts() {
        let temp_dir = tempfile::tempdir().expect("create temp data dir");
        let db = Arc::new(create_test_database().expect("create in-memory db"));
        let app_context = make_test_app_context(temp_dir.path(), Arc::clone(&db));

        // Seed a few user rows with *real* serialized `DataContract` bytes
        // so `Database::get_contracts` round-trips them through
        // `DataContract::versioned_deserialize` instead of silently
        // dropping them. Without this, pagination paths that cross the
        // system/DB boundary would never exercise the merge — they would
        // only ever compare the 5 cached system contracts.
        //
        // Insertion order is intentionally non-ascending so the ordering
        // assertions below prove the listing sorts by `contract_id` at the
        // SQL layer rather than coincidentally returning insertion order.
        let user_ids: Vec<Identifier> = [0xE2u8, 0xE0, 0xE1]
            .into_iter()
            .map(|byte| {
                let id = identifier_with_byte(byte);
                insert_real_contract_row(&app_context, &db, id, None);
                id
            })
            .collect();
        let mut user_ids_sorted = user_ids.clone();
        user_ids_sorted.sort();

        // For a handful of (limit, offset) combinations, the Arc-backed
        // listing must yield the same id sequence as the owned listing.
        let cases: &[(Option<u32>, Option<u32>)] = &[
            (None, None),
            (Some(2), None),
            (Some(SYSTEM_COUNT), None),
            (Some(SYSTEM_COUNT + 1), None),
            (Some(2), Some(3)),
            (Some(2), Some(SYSTEM_COUNT)),
            (None, Some(SYSTEM_COUNT)),
        ];

        for (limit, offset) in cases.iter().copied() {
            let owned: Vec<Identifier> = app_context
                .get_contracts(limit, offset)
                .expect("get_contracts")
                .into_iter()
                .map(|qc| qc.contract.id())
                .collect();
            let arced: Vec<Identifier> = app_context
                .get_contracts_arc(limit, offset)
                .expect("get_contracts_arc")
                .into_iter()
                .map(|qc| qc.contract.id())
                .collect();
            assert_eq!(
                owned, arced,
                "pagination (limit={limit:?}, offset={offset:?}) diverges between get_contracts and get_contracts_arc",
            );
        }

        // Direct proof that the seeded DB-backed rows actually flow
        // through the merge: the no-pagination listing must contain every
        // system contract plus every user contract we inserted.
        let full = app_context
            .get_contracts(None, None)
            .expect("get_contracts(None, None) succeeds");
        assert_eq!(
            full.len(),
            SYSTEM_COUNT as usize + user_ids.len(),
            "full listing must contain all system + DB-backed contracts",
        );
        let full_ids: std::collections::HashSet<Identifier> =
            full.iter().map(|qc| qc.contract.id()).collect();
        for id in &user_ids {
            assert!(
                full_ids.contains(id),
                "DB-backed contract {id:?} should appear in the full listing",
            );
        }

        // Cross-boundary page: offset=4 (last system contract), limit=4 →
        // 1 system + 3 DB-backed rows. Exercises the residual
        // `db_offset` / `db_limit` math end-to-end.
        let crossed = app_context
            .get_contracts(Some(4), Some(SYSTEM_COUNT - 1))
            .expect("cross-boundary pagination succeeds");
        assert_eq!(
            crossed.len(),
            4,
            "cross-boundary page must return one system + three DB-backed rows",
        );
        // The DB-backed tail (positions 1..=3) must come back in
        // `contract_id`-ascending order regardless of the order rows were
        // inserted. This is what makes LIMIT/OFFSET pagination deterministic
        // across calls.
        let crossed_db_ids: Vec<Identifier> =
            crossed[1..].iter().map(|qc| qc.contract.id()).collect();
        assert_eq!(
            crossed_db_ids, user_ids_sorted,
            "DB-backed contracts must appear in contract_id-ascending order in the cross-boundary page",
        );

        // Slicing the DB-backed tail into two consecutive pages must yield
        // disjoint, in-order halves whose concatenation reproduces the full
        // sorted sequence. Without a deterministic `ORDER BY`, these pages
        // could overlap or skip rows.
        let page_one = app_context
            .get_contracts(Some(2), Some(SYSTEM_COUNT))
            .expect("DB page one");
        let page_two = app_context
            .get_contracts(Some(2), Some(SYSTEM_COUNT + 2))
            .expect("DB page two");
        let page_one_ids: Vec<Identifier> = page_one.iter().map(|qc| qc.contract.id()).collect();
        let page_two_ids: Vec<Identifier> = page_two.iter().map(|qc| qc.contract.id()).collect();
        assert_eq!(
            page_one_ids,
            user_ids_sorted[..2],
            "first DB page must contain the two smallest contract_ids in ascending order",
        );
        assert_eq!(
            page_two_ids,
            user_ids_sorted[2..],
            "second DB page must contain the remaining contract_ids in ascending order",
        );
    }
}
