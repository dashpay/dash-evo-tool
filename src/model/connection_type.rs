use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum ConnectionType {
    #[default]
    DashCore,
    DashSpv,
}

impl ConnectionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionType::DashCore => "Dash Core",
            ConnectionType::DashSpv => "Dash SPV",
        }
    }
}
