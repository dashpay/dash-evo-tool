use std::collections::BTreeMap;

use crate::{context::AppContext, model::qualified_identity::DPNSNameInfo};
use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters, document_type::accessors::DocumentTypeV0Getters,
        },
        document::{DocumentV0, DocumentV0Getters},
        identity::accessors::IdentityGettersV0,
        platform_value::{Bytes32, Value},
        util::{hash::hash_double, strings::convert_to_homograph_safe_chars},
    },
    drive::query::{WhereClause, WhereOperator},
    platform::{transition::put_document::PutDocument, Document, DocumentQuery, FetchMany},
    Sdk,
};
use rand::{rngs::StdRng, Rng, SeedableRng};

use super::{BackendTaskSuccessResult, RegisterDpnsNameInput};
impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        sdk: &Sdk,
        input: RegisterDpnsNameInput,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut rng = StdRng::from_entropy();
        let dpns_contract = self.dpns_contract.clone();

        let mut qualified_identity = input.qualified_identity;

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

        let _ = preorder_document
            .put_to_platform_and_wait_for_response(
                sdk,
                preorder_document_type.to_owned_document_type(),
                entropy.0,
                public_key.clone(),
                self.dpns_contract.clone(),
                &qualified_identity,
            )
            .await
            .map_err(|e| e.to_string())?;

        let _ = domain_document
            .put_to_platform_and_wait_for_response(
                sdk,
                domain_document_type.to_owned_document_type(),
                entropy.0,
                public_key.clone(),
                self.dpns_contract.clone(),
                &qualified_identity,
            )
            .await
            .map_err(|e| e.to_string())?;

        // Re-fetch the identity's DPNS names from Platform
        // TODO: Use the proof in the response to see if the name is contested or not (document is returned whether it's contested or not)
        let dpns_names_document_query = DocumentQuery {
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(qualified_identity.identity.id().into()),
            }],
            order_by_clauses: vec![],
            limit: 100,
            start: None,
        };

        let owned_dpns_names = Document::fetch_many(&self.sdk, dpns_names_document_query)
            .await
            .map(|document_map| {
                document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc
                                .get("normalizedLabel")
                                .map(|label| label.to_str().unwrap_or_default());
                            let acquired_at = doc
                                .created_at()
                                .into_iter()
                                .chain(doc.transferred_at())
                                .max();

                            match (name, acquired_at) {
                                (Some(name), Some(acquired_at)) => Some(DPNSNameInfo {
                                    name: name.to_string(),
                                    acquired_at,
                                }),
                                _ => None,
                            }
                        })
                    })
                    .collect::<Vec<DPNSNameInfo>>()
                    .into()
            })
            .map_err(|e| format!("Error fetching DPNS names: {}", e))?;

        qualified_identity.dpns_names = owned_dpns_names;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity)
            .map_err(|e| format!("Database error: {}", e))?;

        Ok(BackendTaskSuccessResult::Message(
            "Successfully registered dpns name".to_string(),
        ))
    }
}
