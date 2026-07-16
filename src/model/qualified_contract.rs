use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::platform::DataContract;

#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedContract {
    pub contract: DataContract,
    pub alias: Option<String>,
}

/// Token-insertion policy for [`AppContext::insert_contract_if_not_exists`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum InsertTokensToo {
    AllTokensShouldBeAdded,
    NoTokensShouldBeAdded,
    SomeTokensShouldBeAdded(Vec<TokenContractPosition>),
}
