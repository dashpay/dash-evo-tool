use dash_sdk::platform::DataContract;

#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedContract {
    pub contract: DataContract,
    pub alias: Option<String>,
}
