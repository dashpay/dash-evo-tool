use dash_evo_tool::database::Database;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::v0::IdentityV0;
use dash_sdk::dpp::identity::{Identity, IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use serde_json::{Value, json};
use std::{collections::BTreeMap, error::Error, fs, path::PathBuf};

fn text<'a>(v: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    v[field]
        .as_str()
        .ok_or_else(|| format!("missing string {field}").into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().ok_or("expected source directory")?);
    let output = PathBuf::from(args.next().ok_or("expected output directory")?);
    if args.next().is_some() {
        return Err("unexpected extra argument".into());
    }
    fs::create_dir_all(&output)?;
    let path = output.join("data.db");
    if path.exists() {
        return Err("output database already exists".into());
    }
    let db = Database::new(&path)?;
    db.initialize(&path)?;
    db.execute("UPDATE settings SET network = 'testnet' WHERE id = 1", [])?;
    let mut expected = Vec::new();
    for (file, identity_type, alias) in [
        ("user.json", IdentityType::User, "public-dpns-user"),
        ("masternode.json", IdentityType::Evonode, "public-evonode"),
    ] {
        let input: Value = serde_json::from_slice(&fs::read(source.join(file))?)?;
        if file == "masternode.json" {
            let validators: Value =
                serde_json::from_slice(&fs::read(source.join("validator.json"))?)?;
            let validator = &validators["resultSet"][0];
            assert_eq!(validator["identity"], input["identifier"]);
            assert_eq!(validator["proTxInfo"]["type"], "Evo");
        }
        let id = Identifier::from_string(text(&input, "identifier")?, Encoding::Base58)?;
        let mut keys = BTreeMap::new();
        let mut expected_keys = Vec::new();
        for key in input["publicKeys"].as_array().ok_or("missing publicKeys")? {
            let contract_bounds = if key["contractBounds"].is_null() {
                None
            } else {
                let bounds = &key["contractBounds"];
                let id = Identifier::from_string(text(bounds, "identifier")?, Encoding::Base58)?;
                Some(match bounds["documentTypeName"].as_str() {
                    Some(name) => ContractBounds::SingleContractDocumentType {
                        id,
                        document_type_name: name.into(),
                    },
                    None => ContractBounds::SingleContract { id },
                })
            };
            let purpose = match text(key, "purpose")? {
                "AUTHENTICATION" => Purpose::AUTHENTICATION,
                "TRANSFER" => Purpose::TRANSFER,
                "OWNER" => Purpose::OWNER,
                "ENCRYPTION" => Purpose::ENCRYPTION,
                "DECRYPTION" => Purpose::DECRYPTION,
                other => return Err(format!("unsupported purpose {other}").into()),
            };
            let security_level = match text(key, "securityLevel")? {
                "MASTER" => SecurityLevel::MASTER,
                "CRITICAL" => SecurityLevel::CRITICAL,
                "HIGH" => SecurityLevel::HIGH,
                "MEDIUM" => SecurityLevel::MEDIUM,
                other => return Err(format!("unsupported security level {other}").into()),
            };
            let key_type = match text(key, "keyType")? {
                "ECDSA_HASH160" => KeyType::ECDSA_HASH160,
                "ECDSA_SECP256K1" => KeyType::ECDSA_SECP256K1,
                other => return Err(format!("unsupported key type {other}").into()),
            };
            let key_id = u32::try_from(key["keyId"].as_u64().ok_or("invalid keyId")?)?;
            let disabled_at = if key["disabledAt"].is_null() {
                None
            } else {
                Some(key["disabledAt"].as_u64().ok_or("invalid disabledAt")?)
            };
            let public_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: key_id,
                purpose,
                security_level,
                contract_bounds: contract_bounds.clone(),
                key_type,
                read_only: key["readOnly"].as_bool().ok_or("invalid readOnly")?,
                data: hex::decode(text(key, "data")?)?.into(),
                disabled_at,
            });
            expected_keys.push(json!({"id": key_id, "key_type": key["keyType"], "purpose": key["purpose"], "security_level": key["securityLevel"], "data": key["data"], "read_only": key["readOnly"], "disabled_at": key["disabledAt"], "contract_bounds": serde_json::to_value(&contract_bounds)?}));
            if keys.insert(key_id, public_key).is_some() {
                return Err("duplicate keyId".into());
            }
        }
        let dpns_names = if file == "user.json" {
            let domain: Value = serde_json::from_slice(&fs::read(source.join("domain.json"))?)?;
            let data: Value = serde_json::from_str(text(&domain, "data")?)?;
            assert_eq!(domain["documentTypeName"], "domain");
            assert_eq!(domain["transitionType"], "DOCUMENT_CREATE");
            assert_eq!(domain["revision"], 1);
            assert_eq!(data["records"]["identity"], input["identifier"]);
            assert_eq!(domain["owner"]["identifier"], input["identifier"]);
            let acquired_at = u64::try_from(
                chrono::DateTime::parse_from_rfc3339(text(&domain, "timestamp")?)?
                    .timestamp_millis(),
            )?;
            vec![DPNSNameInfo {
                name: text(&data, "label")?.into(),
                acquired_at,
            }]
        } else {
            Vec::new()
        };
        let qi = QualifiedIdentity {
            identity: Identity::V0(IdentityV0 {
                id,
                public_keys: keys,
                balance: text(&input, "balance")?.parse()?,
                revision: text(&input, "revision")?.parse()?,
            }),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: Some(alias.into()),
            private_keys: KeyStorage::default(),
            dpns_names,
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        let bytes = qi.to_bytes();
        assert_eq!(qi, QualifiedIdentity::from_bytes(&bytes));
        assert!(qi.private_keys.private_keys.is_empty());
        db.execute("INSERT INTO identity (id,data,is_local,alias,identity_type,network,status) VALUES (?, ?, 1, ?, ?, 'testnet', 2)", rusqlite::params![id.to_vec(), bytes, alias, format!("{identity_type:?}")])?;
        expected.push(json!({
            "id": input["identifier"], "alias": alias, "identity_type": format!("{identity_type:?}"),
            "network": "testnet", "status": "Active", "balance_credits": text(&input, "balance")?.parse::<u64>()?, "revision": text(&input, "revision")?.parse::<u64>()?,
            "public_keys": expected_keys, "private_key_count": 0,
            "dpns_names": qi.dpns_names.iter().map(|n| json!({"name": n.name, "acquired_at": n.acquired_at})).collect::<Vec<_>>()
        }));
    }
    drop(db);
    let conn = rusqlite::Connection::open(&path)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM identity", [], |r| r.get(0))?;
    let wallets: i64 = conn.query_row("SELECT COUNT(*) FROM wallet", [], |r| r.get(0))?;
    assert_eq!((count, wallets), (2, 0));
    fs::write(
        output.join("expected.json"),
        serde_json::to_string_pretty(&json!({"identities": expected}))?,
    )?;
    println!("Validated {count} public identities, {wallets} wallets, no private keys");
    Ok(())
}
