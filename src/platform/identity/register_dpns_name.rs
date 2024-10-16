use std::collections::BTreeMap;

use crate::{context::AppContext, model::qualified_identity::QualifiedIdentity};
use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters, document_type::accessors::DocumentTypeV0Getters,
        },
        document::DocumentV0,
        identity::accessors::IdentityGettersV0,
        platform_value::Bytes32,
        state_transition::documents_batch_transition::{
            methods::v0::DocumentsBatchTransitionMethodsV0, DocumentsBatchTransition,
        },
        util::{hash::hash_double, strings::convert_to_homograph_safe_chars},
    },
    platform::{
        transition::{
            broadcast::BroadcastStateTransition, put_document::PutDocument,
            put_settings::PutSettings,
        },
        Document,
    },
    RequestSettings, Sdk,
};
use rand::{rngs::StdRng, Rng, SeedableRng};

use super::RegisterDpnsNameInput;
impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        sdk: &Sdk,
        input: RegisterDpnsNameInput,
    ) -> Result<(), String> {
        let mut rng = StdRng::from_entropy();
        let dpns_contract = self.dpns_contract.clone();

        let qualified_identity = input.qualified_identity;

        let entropy = Bytes32::random_with_rng(&mut rng);
        let preorder_document_type = dpns_contract
            .document_type_for_name("preorder")
            .map_err(|_| "DPNS preorder document type not found".to_string())?;
        let domain_document_type = dpns_contract
            .document_type_for_name("domain")
            .map_err(|_| "DPNS domain document type not found".to_string())?;

        let preorder_id = Document::generate_document_id_v0(
            &dpns_contract.id(),
            &qualified_identity.identity.id(),
            preorder_document_type.name().as_str(),
            entropy.as_slice(),
        );
        let domain_id = Document::generate_document_id_v0(
            &dpns_contract.id(),
            &qualified_identity.identity.id(),
            domain_document_type.name().as_str(),
            entropy.as_slice(),
        );

        let salt: [u8; 32] = rng.gen();
        let mut salted_domain_buffer: Vec<u8> = vec![];
        salted_domain_buffer.extend(salt);
        salted_domain_buffer
            .extend((convert_to_homograph_safe_chars(&input.name_input) + ".dash").as_bytes());
        let salted_domain_hash = hash_double(salted_domain_buffer);

        let preorder_document = Document::V0(DocumentV0 {
            id: preorder_id,
            owner_id: qualified_identity.identity.id(),
            properties: BTreeMap::from([(
                "saltedDomainHash".to_string(),
                salted_domain_hash.into(),
            )]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        });
        let domain_document = Document::V0(DocumentV0 {
            id: domain_id,
            owner_id: qualified_identity.identity.id(),
            properties: BTreeMap::from([
                ("parentDomainName".to_string(), "dash".into()),
                ("normalizedParentDomainName".to_string(), "dash".into()),
                ("label".to_string(), input.name_input.clone().into()),
                (
                    "normalizedLabel".to_string(),
                    convert_to_homograph_safe_chars(&input.name_input).into(),
                ),
                ("preorderSalt".to_string(), salt.into()),
                (
                    "records".to_string(),
                    BTreeMap::from([(
                        "identity".to_string(),
                        Into::<dash_sdk::dpp::platform_value::Value>::into(
                            qualified_identity.identity.id(),
                        ),
                    )])
                    .into(),
                ),
                (
                    "subdomainRules".to_string(),
                    BTreeMap::from([(
                        "allowSubdomains".to_string(),
                        Into::<dash_sdk::dpp::platform_value::Value>::into(false),
                    )])
                    .into(),
                ),
            ]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        });

        let public_key = qualified_identity
            .document_signing_key(&preorder_document_type)
            .ok_or(
                "Identity doesn't have an authentication key for signing document transitions"
                    .to_string(),
            )?;

        let identity_contract_nonce = match sdk
            .get_identity_contract_nonce(
                qualified_identity.identity.id(),
                dpns_contract.id(),
                true,
                Some(PutSettings {
                    request_settings: RequestSettings::default(),
                    identity_nonce_stale_time_s: Some(0),
                    user_fee_increase: None,
                }),
            )
            .await
        {
            Ok(nonce) => nonce,
            Err(e) => return Err(e.to_string()),
        };

        let preorder_transition =
            DocumentsBatchTransition::new_document_creation_transition_from_document(
                preorder_document.clone(),
                preorder_document_type,
                entropy.0,
                public_key,
                identity_contract_nonce,
                0,
                &qualified_identity,
                &sdk.version(),
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;

        let domain_transition =
            DocumentsBatchTransition::new_document_creation_transition_from_document(
                domain_document.clone(),
                domain_document_type,
                entropy.0,
                public_key,
                identity_contract_nonce + 1,
                0,
                &qualified_identity,
                &sdk.version(),
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;

        preorder_transition
            .broadcast(sdk)
            .await
            .map_err(|e| e.to_string())?;

        let _preorder_document = match <dash_sdk::platform::Document as PutDocument<
            QualifiedIdentity,
        >>::wait_for_response::<'_, '_, '_>(
            &preorder_document,
            sdk,
            preorder_transition,
            dpns_contract.clone().into(),
        )
        .await
        {
            Ok(document) => document,
            Err(e) => {
                return Err(format!("Preorder document failed to process: {e}"));
            }
        };

        domain_transition
            .broadcast(sdk)
            .await
            .map_err(|e| e.to_string())?;

        let _domain_document = match <dash_sdk::platform::Document as PutDocument<
            QualifiedIdentity,
        >>::wait_for_response::<'_, '_, '_>(
            &domain_document,
            sdk,
            domain_transition,
            dpns_contract.into(),
        )
        .await
        {
            Ok(document) => document,
            Err(e) => {
                return Err(format!("Domain document failed to process: {e}"));
            }
        };

        Ok(())
    }
}
