pub mod address_balance_screen;
pub mod contract_visualizer_screen;
pub mod document_visualizer_screen;
pub mod grovestark_screen;
pub mod platform_info_screen;
pub mod proof_visualizer_screen;
pub mod transition_visualizer_screen;

/// The tabs available in the Tools section, shown in the left-hand chooser panel.
#[derive(PartialEq)]
pub enum ToolsSubscreen {
    PlatformInfo,
    AddressBalance,
    TransactionViewer,
    DocumentViewer,
    ProofViewer,
    ContractViewer,
    GroveSTARK,
    DPNS,
}

impl ToolsSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::PlatformInfo => "Platform info",
            Self::AddressBalance => "Address balance",
            Self::TransactionViewer => "Transaction deserializer",
            Self::ProofViewer => "Proof deserializer",
            Self::DocumentViewer => "Document deserializer",
            Self::ContractViewer => "Contract deserializer",
            Self::GroveSTARK => "ZK Proofs",
            Self::DPNS => "DPNS",
        }
    }
}
