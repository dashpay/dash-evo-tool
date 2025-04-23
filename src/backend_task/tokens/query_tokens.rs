//! Execute token query by keyword on Platform

use dash_sdk::{
    dpp::{document::DocumentV0Getters, platform_value::Value},
    drive::query::{WhereClause, WhereOperator},
    platform::{
        proto::get_documents_request::get_documents_request_v0::Start, Document, DocumentQuery,
        FetchMany, Identifier,
    },
    Sdk,
};

use crate::{
    backend_task::BackendTaskSuccessResult, context::AppContext,
    ui::tokens::tokens_screen::ContractDescriptionInfo,
};

impl AppContext {
    /// 1. Fetch all **contractKeywords** docs that match `keyword` from the Search Contract
    /// 2. For every `contractId` found, fetch its **shortDescription** document from the Search Contract
    /// 3. Return the `(contractId, description)` tuples plus the pagination cursor
    pub async fn query_descriptions_by_keyword(
        &self,
        keyword: &str,
        cursor: &Option<Start>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // ── 1. fetch keyword → contractId docs ────────────────────────────────
        let mut kw_query =
            DocumentQuery::new(self.keyword_search_contract.clone(), "contractKeywords")
                .expect("create query");
        kw_query.limit = 100;
        kw_query.start = cursor.clone();
        kw_query = kw_query.with_where(WhereClause {
            field: "keyword".into(),
            operator: WhereOperator::Equal,
            value: Value::Text(keyword.to_owned()),
        });

        let kw_docs = Document::fetch_many(sdk, kw_query.clone())
            .await
            .map_err(|e| format!("Error fetching keyword docs: {e}"))?;

        // store the order for deterministic pagination
        let mut contract_ids: Vec<Identifier> = Vec::with_capacity(kw_docs.len());
        for (_doc_id, doc_opt) in kw_docs.iter() {
            if let Some(doc) = doc_opt {
                if let Some(cid_val) = doc.get("contractId") {
                    contract_ids.push(
                        cid_val
                            .to_identifier()
                            .map_err(|e| format!("Bad contractId: {e}"))?,
                    );
                }
            }
        }

        // Determine next‑page cursor before we start any second‑phase queries
        let has_next_page = kw_docs.len() == 100;
        let next_cursor = if has_next_page {
            kw_docs
                .keys()
                .last()
                .cloned()
                .map(|last_id| Start::StartAfter(last_id.to_buffer().to_vec()))
        } else {
            None
        };

        // ── 2. for every contractId, fetch its shortDescription doc ───────────
        let mut descriptions: Vec<ContractDescriptionInfo> = Vec::with_capacity(contract_ids.len());

        for cid in &contract_ids {
            // build a WHERE contractId == cid query
            let mut desc_query =
                DocumentQuery::new(self.keyword_search_contract.clone(), "shortDescription")
                    .expect("create desc query");
            desc_query.limit = 1; // only one per contract (schema‑unique)
            desc_query = desc_query.with_where(WhereClause {
                field: "contractId".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(cid.to_owned().into()),
            });

            if let Some((_, Some(desc_doc))) = Document::fetch_many(sdk, desc_query)
                .await
                .map_err(|e| format!("Error fetching description doc: {e}"))?
                .into_iter()
                .next()
            {
                if let Some(Value::Text(desc_txt)) = desc_doc.get("description") {
                    descriptions.push(ContractDescriptionInfo {
                        data_contract_id: *cid,
                        description: desc_txt.to_owned(),
                    });
                }
            }
        }

        // ── 3. return result ────────────────────────────────────────────────
        Ok(BackendTaskSuccessResult::DescriptionsByKeyword(
            descriptions,
            next_cursor,
        ))
    }
}
