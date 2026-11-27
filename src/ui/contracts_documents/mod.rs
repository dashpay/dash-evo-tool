use crate::backend_task::{CacheDomain, CacheUpdate};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;

pub mod add_contracts_screen;
pub mod contracts_documents_screen;
pub mod document_action_screen;
pub mod group_actions_screen;
pub mod register_contract_screen;
pub mod update_contract_screen;

fn signing_identities_changed(app_context: &AppContext, update: CacheUpdate) -> bool {
    update.network == app_context.network()
        && matches!(update.domain, CacheDomain::Identities | CacheDomain::All)
}

fn reload_signing_identities(
    app_context: &AppContext,
    selection: &mut String,
) -> Option<(Vec<QualifiedIdentity>, Option<QualifiedIdentity>)> {
    let identities = match app_context.load_local_user_identities() {
        Ok(identities) => identities,
        Err(error) => {
            tracing::warn!(
                ?error,
                "Could not reload stored identities for contract and document forms"
            );
            return None;
        }
    };
    let selected = match Identifier::from_base58_text(selection) {
        Some(id) => identities.iter().find(|qi| qi.identity.id() == id),
        None => identities.first(),
    }
    .cloned();
    if let Some(identity) = &selected {
        *selection = identity.identity.id().to_string(Encoding::Base58);
    }
    Some((identities, selected))
}
