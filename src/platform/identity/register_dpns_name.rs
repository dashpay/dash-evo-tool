use std::collections::{BTreeMap, HashSet};

use crate::{context::AppContext, model::qualified_identity::EncryptedPrivateKeyTarget};
use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters, document_type::accessors::DocumentTypeV0Getters,
        },
        document::{DocumentV0, DocumentV0Getters, DocumentV0Setters},
        identity::{accessors::IdentityGettersV0, KeyType, Purpose, SecurityLevel},
        platform_value::Bytes32,
        state_transition::documents_batch_transition::{
            methods::v0::DocumentsBatchTransitionMethodsV0, DocumentsBatchTransition,
        },
        util::{hash::hash_double, strings::convert_to_homograph_safe_chars},
        version::PlatformVersion,
    },
    platform::{
        transition::{
            broadcast::BroadcastStateTransition, put_document::PutDocument,
            put_settings::PutSettings,
        },
        Document, Identity,
    },
    RequestSettings, Sdk,
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use simple_signer::signer::SimpleSigner;

use super::DpnsNameInputToRegister;
impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        sdk: &Sdk,
        input: DpnsNameInputToRegister,
    ) -> Result<(), String> {
        let mut rng = StdRng::from_entropy();
        let platform_version = PlatformVersion::latest();
        let dpns_contract = self.dpns_contract.clone();

        let qualified_identities = self.load_local_qualified_identities().unwrap_or_default();
        let qualified_identity = qualified_identities
            .iter()
            .find(|identity| identity.identity.id() == input.identity_id_input)
            .expect("Expected to find the identity in qualified identities vec");

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

        let mut preorder_document = Document::V0(DocumentV0 {
            id: preorder_id,
            owner_id: input.identity_id_input,
            properties: BTreeMap::new(),
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
        let mut domain_document = Document::V0(DocumentV0 {
            id: domain_id,
            owner_id: input.identity_id_input,
            properties: BTreeMap::new(),
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

        let salt: [u8; 32] = rng.gen();
        let mut salted_domain_buffer: Vec<u8> = vec![];
        salted_domain_buffer.extend(salt);
        salted_domain_buffer
            .extend((convert_to_homograph_safe_chars(&input.name_input) + ".dash").as_bytes());
        let salted_domain_hash = hash_double(salted_domain_buffer);

        preorder_document.set("saltedDomainHash", salted_domain_hash.into());
        domain_document.set("parentDomainName", "dash".into());
        domain_document.set("normalizedParentDomainName", "dash".into());
        domain_document.set("label", input.name_input.clone().into());
        domain_document.set(
            "normalizedLabel",
            convert_to_homograph_safe_chars(&input.name_input).into(),
        );
        domain_document.set("records.identity", domain_document.owner_id().into());
        domain_document.set("subdomainRules.allowSubdomains", false.into());
        domain_document.set("preorderSalt", salt.into());

        let identity_contract_nonce = match sdk
            .get_identity_contract_nonce(
                input.identity_id_input,
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

        // Get signer from loaded_identity
        // Convert loaded_identity to SimpleSigner
        let signer = {
            let mut new_signer = SimpleSigner::default();
            let Identity::V0(identity_v0) = &qualified_identity.identity;
            for (key_id, public_key) in &identity_v0.public_keys {
                let identity_key_tuple =
                    (EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity, *key_id);
                if let Some(private_key_bytes) = qualified_identity
                    .encrypted_private_keys
                    .get(&identity_key_tuple)
                {
                    new_signer
                        .private_keys
                        .insert(public_key.clone(), private_key_bytes.1.clone());
                }
            }
            new_signer
        };

        let public_key =
            match qualified_identity.identity.get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                HashSet::from([KeyType::ECDSA_SECP256K1, KeyType::BLS12_381]),
                false,
            ) {
                Some(key) => key,
                None => return Err(
                    "Identity doesn't have an authentication key for signing document transitions"
                        .to_string(),
                ),
            };

        let preorder_transition =
            DocumentsBatchTransition::new_document_creation_transition_from_document(
                preorder_document.clone(),
                preorder_document_type,
                entropy.0,
                public_key,
                identity_contract_nonce,
                0,
                &signer,
                &platform_version,
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
                qualified_identity
                    .identity
                    .get_first_public_key_matching(
                        Purpose::AUTHENTICATION,
                        HashSet::from([SecurityLevel::CRITICAL]),
                        HashSet::from([KeyType::ECDSA_SECP256K1, KeyType::BLS12_381]),
                        false,
                    )
                    .expect("expected to get a signing key"),
                identity_contract_nonce + 1,
                0,
                &signer,
                &platform_version,
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;

        preorder_transition
            .broadcast(sdk)
            .await
            .map_err(|e| e.to_string())?;

        let _preorder_document =
            match <dash_sdk::platform::Document as PutDocument<SimpleSigner>>::wait_for_response::<
                '_,
                '_,
                '_,
            >(
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

        let _domain_document =
            match <dash_sdk::platform::Document as PutDocument<SimpleSigner>>::wait_for_response::<
                '_,
                '_,
                '_,
            >(
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
