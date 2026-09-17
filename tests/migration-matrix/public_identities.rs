//! Public-only identity metadata checked before import and after each real CLI boot.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use dash_evo_tool::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use dash_evo_tool::wallet_backend::KV_SCHEMA_VERSION;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{assertions, cli};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExpectedIdentity {
    pub id: String,
    pub alias: Option<String>,
    pub identity_type: String,
    pub status: String,
    pub balance_credits: u64,
    pub revision: u64,
    pub dpns_names: Vec<ExpectedName>,
    pub public_keys: Vec<ExpectedKey>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExpectedName {
    pub name: String,
    pub acquired_at: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExpectedKey {
    pub id: u32,
    pub key_type: String,
    pub purpose: String,
    pub security_level: String,
    pub data: String,
    pub read_only: bool,
    pub disabled_at: Option<u64>,
    pub contract_bounds: Option<Value>,
}

fn check_records(expected: &[ExpectedIdentity], found: &[ExpectedIdentity]) -> Result<(), String> {
    let index = |records: &[ExpectedIdentity]| {
        records
            .iter()
            .map(|record| (record.id.clone(), record.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let expected_by_id = index(expected);
    let found_by_id = index(found);
    if expected_by_id.len() != expected.len() || found_by_id.len() != found.len() {
        return Err("duplicate identity in public metadata".into());
    }
    if expected_by_id != found_by_id {
        return Err(format!(
            "identity metadata changed: expected {expected_by_id:?}, found {found_by_id:?}"
        ));
    }
    Ok(())
}

// Mirrors context::identity_db::StoredQualifiedIdentity, including its bincode field order.
#[derive(Deserialize)]
struct StoredIdentity {
    qi_bytes: Vec<u8>,
    status: u8,
    identity_type: String,
    wallet_hash: Option<[u8; 32]>,
    wallet_index: Option<u32>,
}

fn snapshot(bytes: &[u8], status: u8, identity_type: &str) -> Result<ExpectedIdentity, String> {
    let qi = QualifiedIdentity::from_bytes(bytes)?;
    if !qi.private_keys.is_empty() || !qi.associated_wallets.is_empty() || qi.wallet_index.is_some()
    {
        return Err(
            "public-only identity unexpectedly contains private keys or wallet bindings".into(),
        );
    }
    if qi.associated_voter_identity.is_some()
        || qi.associated_operator_identity.is_some()
        || qi.associated_owner_key_id.is_some()
    {
        return Err(
            "public-only identity unexpectedly contains associated identities or owner binding"
                .into(),
        );
    }
    if qi.identity_type.to_string() != identity_type {
        return Err("identity type differs between stored metadata and blob".into());
    }
    let public_keys = qi
        .identity
        .public_keys()
        .values()
        .map(|key| {
            Ok(ExpectedKey {
                id: key.id(),
                key_type: format!("{:?}", key.key_type()),
                purpose: format!("{:?}", key.purpose()),
                security_level: format!("{:?}", key.security_level()),
                data: hex::encode(key.data().as_slice()),
                read_only: key.read_only(),
                disabled_at: key.disabled_at(),
                // Compare semantic bounds in the historical expectation format: the
                // SDK changed JSON field names without changing the stored meaning.
                contract_bounds: key.contract_bounds().map(|bounds| match bounds {
                    ContractBounds::SingleContract { id } => serde_json::json!({
                        "type": "singleContract", "id": id.to_string(Encoding::Base58)
                    }),
                    ContractBounds::SingleContractDocumentType {
                        id,
                        document_type_name,
                    } => serde_json::json!({
                        "type": "documentType", "id": id.to_string(Encoding::Base58),
                        "document_type_name": document_type_name
                    }),
                }),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ExpectedIdentity {
        id: qi.identity.id().to_string(Encoding::Base58),
        alias: qi.alias,
        identity_type: identity_type.into(),
        status: IdentityStatus::from(status).to_string(),
        balance_credits: qi.identity.balance(),
        revision: qi.identity.revision(),
        dpns_names: qi
            .dpns_names
            .into_iter()
            .map(|name| ExpectedName {
                name: name.name,
                acquired_at: name.acquired_at,
            })
            .collect(),
        public_keys,
    })
}

pub fn check_source(
    expected: &[ExpectedIdentity],
    data_db: &Path,
    scratch: &Path,
) -> Result<(), String> {
    if expected.is_empty() {
        return Ok(());
    }
    let conn = assertions::open_copy(data_db, scratch, "identity-source")?
        .ok_or("public identity fixture has no data.db")?;
    let mut statement = conn.prepare("SELECT id, data, status, identity_type, alias FROM identity WHERE is_local = 1 AND network = 'testnet' ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, u8>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for row in rows {
        let (id, bytes, status, identity_type, alias) = row.map_err(|e| e.to_string())?;
        let record = snapshot(&bytes, status, &identity_type)?;
        if record.id != dash_sdk::dpp::dashcore::base58::encode_slice(&id) || record.alias != alias
        {
            return Err("legacy identity columns disagree with the serialized identity".into());
        }
        found.push(record);
    }
    check_records(expected, &found)
}

pub fn check_migrated(
    expected: &[ExpectedIdentity],
    network_db: &Path,
    scratch: &Path,
    label: &str,
    cli: &cli::DetCli,
    timeout: Duration,
) -> Result<(), String> {
    if expected.is_empty() {
        return Ok(());
    }
    let conn = assertions::open_copy(network_db, scratch, &format!("identities-{label}"))?
        .ok_or("migrated identity database is missing")?;
    let mut statement = conn.prepare("SELECT identity_id, value FROM meta_identity WHERE key = 'det:identity:v1' ORDER BY identity_id")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for row in rows {
        let (id, bytes) = row.map_err(|e| e.to_string())?;
        let Some((&KV_SCHEMA_VERSION, body)) = bytes.split_first() else {
            return Err("unexpected identity k/v schema version".into());
        };
        let (stored, consumed): (StoredIdentity, _) =
            bincode::serde::decode_from_slice(body, bincode::config::standard())
                .map_err(|e| e.to_string())?;
        if consumed != body.len() || stored.wallet_hash.is_some() || stored.wallet_index.is_some() {
            return Err("unexpected data or wallet binding in public identity record".into());
        }
        let record = snapshot(&stored.qi_bytes, stored.status, &stored.identity_type)?;
        if record.id != dash_sdk::dpp::dashcore::base58::encode_slice(&id) {
            return Err("identity storage key differs from its serialized identity".into());
        }
        found.push(record);
    }
    check_records(expected, &found)?;
    let run = cli.run(&["identity-list"], timeout)?;
    assertions::check_boot(&run, "identity-list")?;
    check_listing(expected, &run.json()?)?;
    println!(
        "    {label}: {} identities retain aliases, DPNS, balances, revisions and every public key; identity-list agrees",
        found.len()
    );
    Ok(())
}

fn check_listing(expected: &[ExpectedIdentity], output: &Value) -> Result<(), String> {
    let listed = output["identities"]
        .as_array()
        .ok_or("identity-list has no identities array")?;
    let found: BTreeMap<_, _> = listed
        .iter()
        .map(|identity| (identity["id"].as_str(), identity))
        .collect();
    if listed.len() != expected.len() || found.len() != listed.len() {
        return Err("identity-list contains missing, extra or duplicate identities".into());
    }
    for identity in expected {
        let actual = found
            .get(&Some(identity.id.as_str()))
            .ok_or_else(|| format!("identity-list omitted {}", identity.id))?;
        let wanted = serde_json::json!({"id":identity.id,"alias":identity.alias,"identity_type":identity.identity_type,
            "status":identity.status,"balance_credits":identity.balance_credits,
            "dpns_names":identity.dpns_names.iter().map(|name| &name.name).collect::<Vec<_>>(),
            "wallet_index":null,"wallet_seed_hashes":[]});
        if **actual != wanted {
            return Err(format!(
                "identity-list metadata mismatch: expected {wanted}, found {actual}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ExpectedIdentity {
        serde_json::from_value(serde_json::json!({
            "id": "public-identity", "alias": "Saved alias", "identity_type": "Masternode",
            "status": "Active", "balance_credits": 123, "revision": 2,
            "dpns_names": [{"name": "saved-name", "acquired_at": 456}],
            "public_keys": [{"id": 1, "key_type": "ECDSA_HASH160", "purpose": "OWNER",
                "security_level": "CRITICAL", "data": "aabb", "read_only": true,
                "disabled_at": null, "contract_bounds": null}]
        }))
        .unwrap()
    }

    #[test]
    fn identical_public_metadata_passes() {
        assert!(check_records(&[identity()], &[identity()]).is_ok());
    }

    #[test]
    fn missing_extra_or_duplicate_identities_fail() {
        let expected = vec![identity()];
        assert!(check_records(&expected, &[]).is_err());
        assert!(check_records(&expected, &[identity(), identity()]).is_err());
        let mut other = identity();
        other.id = "unexpected-identity".into();
        assert!(check_records(&expected, &[identity(), other]).is_err());
    }

    #[test]
    fn lost_or_changed_public_metadata_fails() {
        let original = identity();
        let changes: Vec<(&str, Value)> = vec![
            ("alias", Value::Null),
            ("identity_type", "User".into()),
            ("status", "Unknown".into()),
            ("balance_credits", 0.into()),
            ("revision", 0.into()),
            ("dpns_names", serde_json::json!([])),
            ("public_keys", serde_json::json!([])),
        ];
        for (field, value) in changes {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed[field] = value;
            assert!(
                check_records(
                    std::slice::from_ref(&original),
                    &[serde_json::from_value(changed).unwrap()]
                )
                .is_err(),
                "{field}"
            );
        }
        let mut changed = original.clone();
        changed.dpns_names[0].acquired_at += 1;
        assert!(check_records(std::slice::from_ref(&original), &[changed]).is_err());
        for field in [
            "id",
            "key_type",
            "purpose",
            "security_level",
            "data",
            "read_only",
            "disabled_at",
            "contract_bounds",
        ] {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed["public_keys"][0][field] = match field {
                "id" | "disabled_at" => 7.into(),
                "read_only" => false.into(),
                "contract_bounds" => serde_json::json!({"SingleContract": [1]}),
                _ => "changed".into(),
            };
            assert!(
                check_records(
                    std::slice::from_ref(&original),
                    &[serde_json::from_value(changed).unwrap()]
                )
                .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn cli_listing_must_preserve_metadata_and_remain_wallet_free() {
        let expected = vec![identity()];
        let entry = serde_json::json!({
            "id":"public-identity", "alias":"Saved alias", "identity_type":"Masternode",
            "status":"Active", "balance_credits":123, "dpns_names":["saved-name"],
            "wallet_index":null, "wallet_seed_hashes":[]
        });
        assert!(
            check_listing(
                &expected,
                &serde_json::json!({"identities":[entry.clone()]})
            )
            .is_ok()
        );
        assert!(check_listing(&expected, &serde_json::json!({"identities":[]})).is_err());
        assert!(
            check_listing(
                &expected,
                &serde_json::json!({"identities":[entry.clone(),entry.clone()]})
            )
            .is_err()
        );
        for (field, value) in [
            ("dpns_names", serde_json::json!([])),
            ("wallet_index", 0.into()),
            (
                "wallet_seed_hashes",
                serde_json::json!(["unexpected-wallet"]),
            ),
            ("id", "other".into()),
        ] {
            let mut changed = entry.clone();
            changed[field] = value;
            assert!(
                check_listing(&expected, &serde_json::json!({"identities":[changed]})).is_err(),
                "{field}"
            );
        }
    }
}
