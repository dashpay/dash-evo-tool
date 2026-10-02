use dash_evo_tool::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::version::v12::PLATFORM_V12;
use dash_sdk::drive::drive::Drive;
use dash_sdk::drive::grovedb::operations::proof::GroveDBProof;

#[test]
fn verifies_platform_v1_proof_and_rejects_truncated_proof() {
    let bytes = hex::decode(include_str!("fixtures/grovedb-v1.hex").trim()).unwrap();
    let (proof, consumed): (GroveDBProof, _) =
        bincode::decode_from_slice(&bytes, bincode::config::standard().with_big_endian()).unwrap();
    assert_eq!(consumed, bytes.len());
    assert!(matches!(proof, GroveDBProof::V1(_)));
    let (_, contract) = Drive::verify_contract(&bytes, None, false, false, [0; 32], &PLATFORM_V12)
        .expect("Platform V1 proof must verify");
    assert!(contract.is_none());
    assert!(
        Drive::verify_contract(
            &bytes[..bytes.len() - 1],
            None,
            false,
            false,
            [0; 32],
            &PLATFORM_V12
        )
        .is_err()
    );
}

#[test]
fn v0_9_3_identities_decode_without_data_loss() {
    let expected = [
        (
            "51iVVoMU8ANbxDv7wySh5Sk6mZGSEMpCkeqtkr24mHdE",
            "public-dpns-user",
            IdentityType::User,
            6,
        ),
        (
            "PJUBWbXWmzEYCs99rAAbnCiHRzrnhKLQrXbmSsuPBYB",
            "public-evonode",
            IdentityType::Evonode,
            2,
        ),
    ];
    let fixtures: Vec<_> = include_str!("fixtures/v0.9.3-identities.hex")
        .lines()
        .collect();
    assert_eq!(fixtures.len(), expected.len());
    for (fixture, (id, alias, identity_type, key_count)) in fixtures.into_iter().zip(expected) {
        let bytes = hex::decode(fixture).unwrap();
        let (decoded, consumed): (QualifiedIdentity, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded.identity.id().to_string(Encoding::Base58), id);
        assert_eq!(decoded.alias.as_deref(), Some(alias));
        assert_eq!(decoded.identity_type, identity_type);
        assert_eq!(decoded.identity.public_keys().len(), key_count);
        assert_eq!(decoded.to_bytes(), bytes);
    }
}

#[tokio::test]
async fn identity_signer_preserves_signatures_and_witnesses() {
    use dash_evo_tool::model::qualified_identity::PrivateKeyTarget;
    use dash_sdk::dpp::address_funds::AddressWitness;
    use dash_sdk::dpp::dashcore::{Network, PrivateKey, secp256k1::Secp256k1, signer};
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::signer::Signer;
    use dash_sdk::dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};

    let bytes = hex::decode(
        include_str!("fixtures/v0.9.3-identities.hex")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let mut identity = QualifiedIdentity::from_bytes(&bytes);
    let secret = [42; 32];
    let private = PrivateKey::from_byte_array(&secret, Network::Testnet).unwrap();
    let public = private.public_key(&Secp256k1::new());
    let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 123,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::MASTER,
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: public.to_bytes().into(),
        disabled_at: None,
        contract_bounds: None,
    });
    assert!(identity.sign(&key, b"test").await.is_err());
    identity.private_keys.insert_non_encrypted(
        (PrivateKeyTarget::PrivateKeyOnMainIdentity, 123),
        (key.clone().into(), secret),
    );
    let signature = identity.sign(&key, b"test").await.unwrap();
    signer::verify_data_signature(b"test", signature.as_slice(), &public.to_bytes()).unwrap();
    assert_eq!(
        signature.as_slice(),
        signer::sign(b"test", &secret).unwrap()
    );
    assert_eq!(
        identity.sign_create_witness(&key, b"test").await.unwrap(),
        AddressWitness::P2pkh { signature }
    );
}
