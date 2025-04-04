//! Execute token query by keyword on Platform

use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::{v0::DataContractV0Getters, v1::DataContractV1Getters},
            associated_token::{
                token_configuration::accessors::v0::TokenConfigurationV0Getters,
                token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters,
            },
        },
        document::DocumentV0Getters,
        platform_value::Value,
    },
    drive::query::{WhereClause, WhereOperator},
    platform::{
        proto::get_documents_request::get_documents_request_v0::Start, DataContract, Document,
        DocumentQuery, FetchMany, Identifier,
    },
    query_types::IndexMap,
    Sdk,
};

use crate::{
    backend_task::BackendTaskSuccessResult, context::AppContext,
    ui::tokens::tokens_screen::TokenInfo,
};

impl AppContext {
    /// First we query the search contract by keyword to get the documents with keyword and contractId
    /// Then we do a DataContract::fetch_many using a vector of contract IDs
    /// Then we return the tokens for each contract
    pub async fn query_tokens_by_keyword(
        &self,
        keyword: &String,
        cursor: &Option<Start>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // First query the documents from the search contract to get the contract IDs
        let mut document_query =
            DocumentQuery::new(self.keyword_search_contract.clone(), "contract")
                .expect("Expected to create a new DocumentQuery");

        document_query.limit = 100;
        document_query.start = cursor.clone();
        document_query = document_query.with_where(WhereClause {
            field: "keyword".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Text(keyword.clone()),
        });

        // Initialize an empty IndexMap to accumulate documents for this page
        let mut page_docs: IndexMap<Identifier, Option<Document>> = IndexMap::new();

        // Fetch a single page
        let docs_batch_result = Document::fetch_many(sdk, document_query.clone())
            .await
            .map_err(|e| format!("Error fetching documents: {}", e))?;

        let batch_len = docs_batch_result.len();

        // Insert the batch into the page map
        for (id, doc_opt) in docs_batch_result {
            page_docs.insert(id, doc_opt);
        }

        // Determine if there's a next page
        let has_next_page = batch_len == 100;

        // If there's a next page, set the 'start' parameter for the next cursor, to be returned
        let next_cursor = if has_next_page {
            page_docs.keys().last().cloned().map(|last_doc_id| {
                let id_bytes = last_doc_id.to_buffer();
                Start::StartAfter(id_bytes.to_vec())
            })
        } else {
            None
        };

        // Now we have the docs, extract the contract IDs and query each contract for its tokens
        let mut discovered_tokens: Vec<TokenInfo> = Vec::new();
        let contract_ids = page_docs
            .iter()
            .filter_map(|(_, doc_opt)| {
                if let Some(doc) = doc_opt {
                    // Extract the contract ID from the document
                    if let Some(contract_id) = doc.get("contractId") {
                        return Some(
                            contract_id
                                .to_identifier()
                                .expect("Expected to get convert ID"),
                        );
                    }
                }
                None
            })
            .collect::<Vec<Identifier>>();

        // Fetch the contracts using the contract IDs
        let contracts_batch_result = DataContract::fetch_many(sdk, contract_ids)
            .await
            .map_err(|e| format!("Error fetching contracts: {}", e))?;

        // Iterate over the contracts and extract the tokens
        for (_, contract_opt) in contracts_batch_result {
            if let Some(contract) = contract_opt {
                // Extract the tokens from the contract
                let tokens = contract.tokens();
                for token in tokens {
                    let token_info = TokenInfo {
                        token_identifier: contract
                            .token_id(*token.0)
                            .expect("Expected to get token ID"),
                        token_name: token
                            .1
                            .conventions()
                            .plural_form_by_language_code_or_default("en")
                            .to_string(),
                        token_position: *token.0,
                        data_contract_id: contract.id(),
                    };
                    // Create an IdentityTokenBalance object and add it to the discovered tokens
                    discovered_tokens.push(token_info);
                }
            }
        }

        // Return the discovered tokens and the next cursor
        Ok(BackendTaskSuccessResult::TokensByKeyword(
            discovered_tokens,
            next_cursor,
        ))
    }
}
