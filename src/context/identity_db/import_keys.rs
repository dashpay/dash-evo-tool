//! Durable inventory of identity keys retained across interrupted imports.

use super::*;
use std::collections::BTreeSet;

/// Global storage survives identity removal until its vault cleanup completes.
const IMPORT_KEYS_PREFIX: &str = "det:identity_import_keys:v1:";

fn import_keys_key(id: &Identifier) -> String {
    format!("{IMPORT_KEYS_PREFIX}{}", id.to_string(Encoding::Base58))
}

impl AppContext {
    /// Read all import placements, including keys absent from the identity blob.
    pub(crate) fn retained_identity_import_keys(
        &self,
        id: &Identifier,
    ) -> Result<BTreeSet<(PrivateKeyTarget, KeyID)>, TaskError> {
        let stored: Vec<(StoredPrivateKeyTarget, KeyID)> = self
            .det_kv()?
            .get(DetScope::Global, &import_keys_key(id))
            .map_err(identity_err)?
            .unwrap_or_default();
        Ok(stored
            .into_iter()
            .map(|(target, key)| (target.into(), key))
            .collect())
    }

    /// Record intended placements before writing secrets, under the record lock.
    /// Keep omitted placements until explicit identity removal cleans the vault.
    pub(crate) fn record_identity_import_keys(
        &self,
        id: &Identifier,
        keys: &BTreeSet<(PrivateKeyTarget, KeyID)>,
    ) -> Result<(), TaskError> {
        let mut retained = self.retained_identity_import_keys(id)?;
        let before = retained.len();
        retained.extend(keys.iter().cloned());
        if retained.len() == before {
            return Ok(());
        }
        let stored: Vec<(StoredPrivateKeyTarget, KeyID)> = retained
            .into_iter()
            .map(|(target, key)| (target.into(), key))
            .collect();
        self.det_kv()?
            .put(DetScope::Global, &import_keys_key(id), &stored)
            .map_err(identity_err)
    }

    /// Retire the inventory only after all retained vault keys have been deleted.
    pub(super) fn forget_identity_import_keys(&self, id: &Identifier) -> Result<(), TaskError> {
        self.det_kv()?
            .delete(DetScope::Global, &import_keys_key(id))
            .map_err(identity_err)
    }
}
