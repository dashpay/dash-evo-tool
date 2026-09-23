//! State transitions built the way DET builds them on a protocol 13 network
//! (mainnet and testnet today): only encodings protocol 13 decodes.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::address_funds::AddressWitness;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::config::DataContractConfig;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::dpp::data_contract::document_type::action_fees::agreement::DocumentActionFeeAgreement;
use dash_sdk::dpp::data_contract::document_type::action_fees::agreement::v0::DocumentActionFeeAgreementV0;
use dash_sdk::dpp::document::{Document, DocumentV0};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::identity::{Identity, IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::{BinaryData, Identifier, platform_value};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::state_transition::batch_transition::BatchTransition;
use dash_sdk::dpp::state_transition::batch_transition::batched_transition::BatchedTransition;
use dash_sdk::dpp::state_transition::batch_transition::batched_transition::document_transition::DocumentTransitionV0Methods;
use dash_sdk::dpp::state_transition::batch_transition::document_base_transition::DocumentBaseTransition;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::v0::DocumentsBatchTransitionMethodsV0;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::public_key_in_creation::IdentityPublicKeyInCreation;
use dash_sdk::dpp::version::PlatformVersion;

use crate::context::AppContext;
use crate::context::test_support::test_app_context_for_network;

/// Signs everything with placeholder bytes: these tests check what is
/// encoded, not signature validity.
#[derive(Debug)]
struct PlaceholderSigner;

#[async_trait]
impl Signer<IdentityPublicKey> for PlaceholderSigner {
    async fn sign(
        &self,
        _key: &IdentityPublicKey,
        _data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        Ok(BinaryData::new(vec![0; 65]))
    }

    async fn sign_create_witness(
        &self,
        _key: &IdentityPublicKey,
        _data: &[u8],
    ) -> Result<AddressWitness, ProtocolError> {
        Err(ProtocolError::Generic(
            "no address witnesses here".to_string(),
        ))
    }

    fn can_sign_with(&self, _key: &IdentityPublicKey) -> bool {
        true
    }
}

/// A mainnet context, which builds at protocol 13 until the network says
/// otherwise.
fn mainnet_context() -> (tempfile::TempDir, Arc<AppContext>, &'static PlatformVersion) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ctx = test_app_context_for_network(tmp.path(), Network::Mainnet);
    let platform_version = ctx.platform_version();
    assert_eq!(platform_version.protocol_version, 13);
    (tmp, ctx, platform_version)
}

fn plain_key(id: u32, security_level: SecurityLevel) -> IdentityPublicKey {
    IdentityPublicKey::from(IdentityPublicKeyV0 {
        id,
        purpose: Purpose::AUTHENTICATION,
        security_level,
        contract_bounds: None,
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: vec![2; 33].into(),
        disabled_at: None,
    })
}

/// A key without limits, added on a protocol 13 network, goes out as the
/// version 0 key encoding protocol 13 decodes.
#[tokio::test]
async fn a_plain_key_is_added_in_the_protocol_13_encoding() {
    let (_tmp, _ctx, platform_version) = mainnet_context();
    let mut identity = Identity::create_basic_identity(Identifier::from([4; 32]), platform_version)
        .expect("identity");
    identity.add_public_key(plain_key(0, SecurityLevel::MASTER));

    let transition = IdentityUpdateTransition::try_from_identity_with_signer(
        &identity,
        &0,
        vec![plain_key(1, SecurityLevel::HIGH)],
        vec![],
        1,
        UserFeeIncrease::default(),
        &PlaceholderSigner,
        platform_version,
        None,
    )
    .await
    .expect("identity update");

    let StateTransition::IdentityUpdate(IdentityUpdateTransition::V0(update)) = transition else {
        panic!("expected a version 0 identity update, got {transition:?}");
    };
    assert_eq!(update.add_public_keys.len(), 1);
    assert!(matches!(
        update.add_public_keys[0],
        IdentityPublicKeyInCreation::V0(_)
    ));
}

fn note_contract(platform_version: &PlatformVersion) -> dash_sdk::platform::DataContract {
    use dash_sdk::dpp::data_contract::v1::DataContractV1;

    let id = Identifier::from([8; 32]);
    let config = DataContractConfig::default_for_version(platform_version).expect("config");
    let schema = platform_value!({
        "type": "object",
        "properties": {"name": {"type": "string", "position": 0, "maxLength": 63_u32}},
        "required": ["name"],
        "additionalProperties": false,
    });
    let note = DocumentType::try_from_schema(
        id,
        1,
        config.version(),
        "note",
        schema,
        None,
        &BTreeMap::new(),
        &config,
        true,
        &mut Vec::new(),
        platform_version,
    )
    .expect("document type");
    dash_sdk::platform::DataContract::V1(DataContractV1 {
        id,
        version: 1,
        owner_id: Identifier::from([7; 32]),
        document_types: BTreeMap::from([("note".to_string(), note)]),
        config,
        schema_defs: None,
        groups: BTreeMap::new(),
        tokens: BTreeMap::new(),
        keywords: vec![],
        created_at: None,
        updated_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        created_at_epoch: None,
        updated_at_epoch: None,
        description: None,
    })
}

fn note(owner_id: Identifier) -> Document {
    Document::V0(DocumentV0 {
        id: Identifier::from([9; 32]),
        owner_id,
        creator_id: None,
        properties: BTreeMap::from([("name".to_string(), "hello".into())]),
        revision: Some(1),
        created_at: None,
        updated_at: None,
        transferred_at: None,
        created_at_block_height: None,
        updated_at_block_height: None,
        transferred_at_block_height: None,
        created_at_core_block_height: None,
        updated_at_core_block_height: None,
        transferred_at_core_block_height: None,
        contract_version: None,
    })
}

async fn create_note(
    platform_version: &PlatformVersion,
    options: Option<StateTransitionCreationOptions>,
) -> Result<StateTransition, Box<ProtocolError>> {
    let contract = note_contract(platform_version);
    let document_type = contract.document_type_for_name("note").expect("note type");
    let owner_id = Identifier::from([4; 32]);
    BatchTransition::new_document_creation_transition_from_document(
        note(owner_id),
        document_type,
        [3; 32],
        &plain_key(1, SecurityLevel::HIGH),
        1,
        UserFeeIncrease::default(),
        None,
        &PlaceholderSigner,
        platform_version,
        options,
    )
    .await
    .map_err(Box::new)
}

/// A document created on a protocol 13 network uses a document base that
/// protocol 13 decodes: not the protocol 14 one that carries a fee agreement.
#[tokio::test]
async fn a_document_is_created_in_the_protocol_13_encoding() {
    let (_tmp, _ctx, platform_version) = mainnet_context();

    let transition = create_note(platform_version, None)
        .await
        .expect("document create");

    let StateTransition::Batch(batch) = transition else {
        panic!("expected a batch, got {transition:?}");
    };
    let base = match &batch {
        BatchTransition::V0(v0) => v0.transitions[0].base().clone(),
        BatchTransition::V1(v1) => match &v1.transitions[0] {
            BatchedTransition::Document(document) => document.base().clone(),
            other => panic!("expected a document transition, got {other:?}"),
        },
    };
    assert!(
        !matches!(base, DocumentBaseTransition::V2(_)),
        "protocol 13 cannot decode {base:?}"
    );
}

/// A fee agreement cannot be encoded for a protocol 13 network: building
/// fails locally instead of producing a transition every node rejects.
#[tokio::test]
async fn a_fee_agreement_is_refused_on_protocol_13() {
    let (_tmp, _ctx, platform_version) = mainnet_context();
    let options = StateTransitionCreationOptions {
        action_fee_agreement: Some(DocumentActionFeeAgreement::V0(
            DocumentActionFeeAgreementV0 {
                owner: 5,
                moderators: 0,
                fee_multiplier: None,
            },
        )),
        ..Default::default()
    };

    assert!(
        options
            .validate_base_carries_action_fee_agreement(platform_version)
            .is_err()
    );
    assert!(create_note(platform_version, Some(options)).await.is_err());
}
