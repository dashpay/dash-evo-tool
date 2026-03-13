use super::AppContext;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::model::contested_name::ContestedName;
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use rusqlite::Result;

impl AppContext {
    /// Inserts a local qualified identity into the database
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletSeedHash, u32)>,
    ) -> Result<()> {
        self.db.insert_local_qualified_identity(
            qualified_identity,
            wallet_and_identity_id_info,
            self,
        )
    }

    /// Updates a local qualified identity in the database
    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> Result<()> {
        self.db
            .update_local_qualified_identity(qualified_identity, self)
    }

    /// Sets the alias for an identity
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> Result<()> {
        self.db.set_identity_alias(identifier, new_alias)
    }

    pub fn set_contract_alias(
        &self,
        contract_id: &Identifier,
        new_alias: Option<&str>,
    ) -> Result<()> {
        self.db.set_contract_alias(contract_id, new_alias)
    }

    /// Gets the alias for an identity
    pub fn get_identity_alias(&self, identifier: &Identifier) -> Result<Option<String>> {
        self.db.get_identity_alias(identifier)
    }

    /// Fetches all local qualified identities from the database
    pub fn load_local_qualified_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        self.db.get_local_qualified_identities(self, &wallets)
    }

    /// Fetches all local qualified identities from the database
    #[allow(dead_code)] // May be used for loading identities in wallets
    pub fn load_local_qualified_identities_in_wallets(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        self.db
            .get_local_qualified_identities_in_wallets(self, &wallets)
    }

    pub fn get_identity_by_id(
        &self,
        identity_id: &Identifier,
    ) -> Result<Option<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        // Get the identity from the database
        let result = self.db.get_identity_by_id(identity_id, self, &wallets)?;

        Ok(result)
    }

    /// Fetches all voting identities from the database
    pub fn load_local_voting_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        self.db.get_local_voting_identities(self)
    }

    /// Fetches all local user identities from the database
    pub fn load_local_user_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let identities = self.db.get_local_user_identities(self)?;

        Ok(identities
            .into_iter()
            .map(|(mut identity, wallet_hash)| {
                if let Some(wallet_id) = wallet_hash {
                    // Load wallets for each identity
                    self.load_wallet_for_identity(
                        &mut identity,
                        &[wallet_id],
                    )
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            identity = %identity.identity.id(),
                            error = ?e,
                            "cannot load wallet for identity when loading local user identities",
                        )
                    })
                } else {
                    tracing::debug!(
                        identity = %identity.identity.id(),
                        "no wallet hash found for identity when loading local user identities",
                    );
                }
                identity
            })
            .collect())
    }

    fn load_wallet_for_identity(
        &self,
        identity: &mut QualifiedIdentity,
        wallet_hashes: &[WalletSeedHash],
    ) -> Result<()> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        for wallet_hash in wallet_hashes {
            if let Some(wallet) = wallets.get(wallet_hash) {
                identity
                    .associated_wallets
                    .insert(*wallet_hash, wallet.clone());
            } else {
                tracing::warn!(
                    wallet = %hex::encode(wallet_hash),
                    identity = %identity.identity.id(),
                    "wallet not found for identity when loading local user identities",
                );
            }
        }

        Ok(())
    }

    /// Fetches all contested names from the database including past and active ones
    pub fn all_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_all_contested_names(self)
    }

    /// Fetches all ongoing contested names from the database
    pub fn ongoing_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_ongoing_contested_names(self)
    }

    /// Inserts scheduled votes into the database
    pub fn insert_scheduled_votes(&self, scheduled_votes: &Vec<ScheduledDPNSVote>) -> Result<()> {
        self.db.insert_scheduled_votes(self, scheduled_votes)
    }

    /// Fetches all scheduled votes from the database
    pub fn get_scheduled_votes(&self) -> Result<Vec<ScheduledDPNSVote>> {
        self.db.get_scheduled_votes(self)
    }

    /// Clears all scheduled votes from the database
    pub fn clear_all_scheduled_votes(&self) -> Result<()> {
        self.db.clear_all_scheduled_votes(self)
    }

    /// Clears all executed scheduled votes from the database
    pub fn clear_executed_scheduled_votes(&self) -> Result<()> {
        self.db.clear_executed_scheduled_votes(self)
    }

    /// Deletes a scheduled vote from the database
    #[allow(clippy::ptr_arg)]
    pub fn delete_scheduled_vote(&self, identity_id: &[u8], contested_name: &String) -> Result<()> {
        self.db
            .delete_scheduled_vote(self, identity_id, contested_name)
    }

    /// Marks a scheduled vote as executed in the database
    pub fn mark_vote_executed(&self, identity_id: &[u8], contested_name: String) -> Result<()> {
        self.db
            .mark_vote_executed(self, identity_id, contested_name)
    }

    /// Fetches the local identities from the database and then maps them to their DPNS names.
    pub fn local_dpns_names(&self) -> Result<Vec<(Identifier, DPNSNameInfo)>> {
        let wallets = self.wallets.read().unwrap_or_else(|e| e.into_inner());
        let qualified_identities = self.db.get_local_qualified_identities(self, &wallets)?;

        // Map each identity's DPNS names to (Identifier, DPNSNameInfo) tuples
        let dpns_names = qualified_identities
            .iter()
            .flat_map(|qualified_identity| {
                qualified_identity.dpns_names.iter().map(|dpns_name_info| {
                    (
                        qualified_identity.identity.id(),
                        DPNSNameInfo {
                            name: dpns_name_info.name.clone(),
                            acquired_at: dpns_name_info.acquired_at,
                        },
                    )
                })
            })
            .collect::<Vec<(Identifier, DPNSNameInfo)>>();

        Ok(dpns_names)
    }
}
