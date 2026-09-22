//! Usage limits of identity authentication keys (protocol version 14).

use std::collections::BTreeMap;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::Sdk;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::platform::identity_keys_remaining_budgets::{
    IdentityKeysRemainingBudgets, IdentityKeysRemainingBudgetsQuery,
};
use dash_sdk::platform::{Fetch, Identifier};

use super::BackendTaskSuccessResult;

impl AppContext {
    /// What is left of the budgets of `key_ids` of `identity_id`.
    ///
    /// One entry per requested key: `Some(credits)` for a budgeted key (zero
    /// means the key can no longer sign), `None` for a key without a budget or
    /// one Platform does not know.
    pub(super) async fn fetch_key_remaining_budgets(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        key_ids: Vec<KeyID>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if key_ids.is_empty() {
            return Ok(BackendTaskSuccessResult::IdentityKeyRemainingBudgets {
                identity_id,
                budgets: BTreeMap::new(),
            });
        }
        let fetched = IdentityKeysRemainingBudgets::fetch(
            sdk,
            IdentityKeysRemainingBudgetsQuery {
                identity_id,
                key_ids: key_ids.clone(),
            },
        )
        .await?
        .unwrap_or_default();

        Ok(BackendTaskSuccessResult::IdentityKeyRemainingBudgets {
            identity_id,
            budgets: remaining_budgets_by_key(&key_ids, &fetched),
        })
    }
}

/// One entry per requested key id, in key id order: the remaining budget
/// Platform reported, `None` when it reported none.
fn remaining_budgets_by_key(
    key_ids: &[KeyID],
    fetched: &IdentityKeysRemainingBudgets,
) -> BTreeMap<KeyID, Option<Credits>> {
    key_ids
        .iter()
        .map(|key_id| (*key_id, fetched.get(key_id).copied().flatten()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_requested_key_gets_an_entry() {
        let fetched: IdentityKeysRemainingBudgets = [(1, Some(500)), (2, None), (3, Some(0))]
            .into_iter()
            .collect();
        let by_key = remaining_budgets_by_key(&[1, 2, 3, 4], &fetched);
        assert_eq!(
            by_key,
            BTreeMap::from([(1, Some(500)), (2, None), (3, Some(0)), (4, None)])
        );
    }
}
