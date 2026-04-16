use crate::context::AppContext;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::sync::Arc;

/// Payment record for local UI display
#[derive(Debug, Clone)]
pub struct PaymentRecord {
    pub id: String,
    pub from_identity: Identifier,
    pub to_identity: Identifier,
    pub from_address: Option<dash_sdk::dpp::dashcore::Address>,
    pub to_address: Option<dash_sdk::dpp::dashcore::Address>,
    pub amount: u64,
    pub tx_id: Option<String>,
    pub memo: Option<String>,
    pub timestamp: u64,
    pub status: PaymentStatus,
    pub address_index: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaymentStatus {
    Pending,
    Broadcast,
    Confirmed(u32),
    Failed(String),
}

/// Load payment history from local database
pub async fn load_payment_history(
    app_context: &Arc<AppContext>,
    identity_id: &Identifier,
    contact_id: Option<&Identifier>,
) -> Result<Vec<PaymentRecord>, String> {
    let stored_payments = app_context
        .db
        .load_payment_history(identity_id, 100)
        .map_err(|e| format!("Failed to load payment history: {}", e))?;

    let mut records = Vec::new();
    for sp in stored_payments {
        let from_id = Identifier::from_bytes(&sp.from_identity_id)
            .map_err(|e| format!("Invalid from_identity_id: {}", e))?;
        let to_id = Identifier::from_bytes(&sp.to_identity_id)
            .map_err(|e| format!("Invalid to_identity_id: {}", e))?;

        if let Some(filter_id) = contact_id
            && from_id != *filter_id
            && to_id != *filter_id
        {
            continue;
        }

        let status = match sp.status.as_str() {
            "confirmed" => PaymentStatus::Confirmed(1),
            "failed" => PaymentStatus::Failed("Transaction failed".to_string()),
            "pending" => PaymentStatus::Pending,
            _ => PaymentStatus::Broadcast,
        };

        records.push(PaymentRecord {
            id: sp.tx_id.clone(),
            from_identity: from_id,
            to_identity: to_id,
            from_address: None,
            to_address: None,
            amount: sp.amount as u64,
            tx_id: Some(sp.tx_id.clone()),
            memo: sp.memo,
            timestamp: sp.created_at as u64,
            status,
            address_index: 0,
        });
    }

    Ok(records)
}
