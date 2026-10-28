//! Building new Platform documents for create transitions.

use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeV0Methods;
use dash_sdk::dpp::document::Document;
use dash_sdk::dpp::platform_value::{Identifier, Value};
use dash_sdk::dpp::version::PlatformVersion;
use std::collections::BTreeMap;

/// Builds a new document of `document_type`, owned by `owner_id`, for a create
/// transition that uses `entropy`.
///
/// Before protocol version 14 the id derived here from the entropy is the
/// document's id, which Platform recomputes from the entropy. From protocol
/// version 14 (`Document::document_id_depends_on_nonce`) the id also commits to
/// the identity contract nonce, so this one is only a placeholder that building
/// the create transition replaces. Read the id from the confirmed document.
///
/// The document type picks the document structure for `platform_version` and
/// keeps `properties` as given. The block heights are placeholders: Platform
/// sets them itself, and the create transition does not carry them.
pub fn new_document_for_creation(
    document_type: DocumentTypeRef<'_>,
    owner_id: Identifier,
    properties: BTreeMap<String, Value>,
    entropy: &[u8; 32],
    platform_version: &PlatformVersion,
) -> Result<Document, Box<ProtocolError>> {
    let id = Document::generate_document_id_v0(
        &document_type.data_contract_id(),
        &owner_id,
        document_type.name(),
        entropy,
    );
    document_type
        .create_document_with_prevalidated_properties(
            id,
            owner_id,
            0,
            0,
            properties,
            platform_version,
        )
        .map_err(Box::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::DataContract;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::document::{DocumentV0, DocumentV0Getters};
    use dash_sdk::dpp::state_transition::batch_transition::batched_transition::document_create_transition::DocumentCreateTransition;
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};

    const OWNER: [u8; 32] = [0x11; 32];
    const ENTROPY: [u8; 32] = [0x22; 32];
    const NONCE: u64 = 7;

    fn contact_request_properties() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("toUserId".to_string(), Value::Identifier([0x33; 32])),
            ("senderKeyIndex".to_string(), Value::U32(2)),
            ("recipientKeyIndex".to_string(), Value::U32(3)),
            ("accountReference".to_string(), Value::U32(0)),
            (
                "encryptedPublicKey".to_string(),
                Value::Bytes(vec![0x44; 96]),
            ),
        ])
    }

    fn contact_info_properties() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("encToUserId".to_string(), Value::Bytes(vec![0x55; 32])),
            ("rootEncryptionKeyIndex".to_string(), Value::U32(1)),
            ("derivationEncryptionKeyIndex".to_string(), Value::U32(0)),
            ("privateData".to_string(), Value::Bytes(vec![0x66; 48])),
        ])
    }

    /// The document the DashPay tasks built by hand before they used the
    /// document type's factory.
    fn hand_built_document(
        contract: &DataContract,
        document_type_name: &str,
        properties: BTreeMap<String, Value>,
    ) -> Document {
        let owner_id = Identifier::new(OWNER);
        DocumentV0 {
            id: Document::generate_document_id_v0(
                &contract.id(),
                &owner_id,
                document_type_name,
                &ENTROPY,
            ),
            owner_id,
            creator_id: None,
            properties,
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
            moderated_at: None,
            moderated_by: None,
        }
        .into()
    }

    fn create_transition(
        contract: &DataContract,
        document_type_name: &str,
        document: Document,
        platform_version: &PlatformVersion,
    ) -> DocumentCreateTransition {
        let document_type = contract
            .document_type_for_name(document_type_name)
            .expect("document type");
        DocumentCreateTransition::from_document(
            document,
            document_type,
            ENTROPY,
            None,
            NONCE,
            platform_version,
            None,
            None,
        )
        .expect("create transition")
    }

    /// DOC-CREATE-001: a document built by the document type's factory yields
    /// the same create transition as the hand-built document it replaces, under
    /// protocol versions 13 and 14, for a mutable (`contactInfo`) and an
    /// immutable (`contactRequest`) DashPay document type.
    #[test]
    fn factory_document_yields_the_hand_built_create_transition() {
        for protocol_version in [13, 14] {
            let platform_version = PlatformVersion::get(protocol_version).expect("version");
            let contract = load_system_data_contract(SystemDataContract::Dashpay, platform_version)
                .expect("dashpay contract");
            for (document_type_name, properties) in [
                ("contactRequest", contact_request_properties()),
                ("contactInfo", contact_info_properties()),
            ] {
                let document_type = contract
                    .document_type_for_name(document_type_name)
                    .expect("document type");
                let built = new_document_for_creation(
                    document_type,
                    Identifier::new(OWNER),
                    properties.clone(),
                    &ENTROPY,
                    platform_version,
                )
                .expect("document");
                let hand_built = hand_built_document(&contract, document_type_name, properties);
                assert_eq!(built.id(), hand_built.id());
                assert_eq!(built.properties(), hand_built.properties());
                assert!(matches!(built, Document::V0(_)));
                // Unlike the hand-built document, the factory fills the
                // required `$createdAt` and gives an immutable document no
                // revision. The create transition carries neither.
                assert!(built.created_at().is_some());
                assert_eq!(
                    built.revision().is_some(),
                    document_type_name == "contactInfo",
                    "{document_type_name} at protocol {protocol_version}",
                );

                assert_eq!(
                    create_transition(&contract, document_type_name, built, platform_version),
                    create_transition(&contract, document_type_name, hand_built, platform_version),
                    "{document_type_name} at protocol {protocol_version}",
                );
            }
        }
    }
}
