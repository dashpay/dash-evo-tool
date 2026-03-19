//! Per-domain MCP tool implementations.

pub mod meta;
pub mod network;
pub mod wallet;

use rmcp::schemars;
use serde::Deserialize;

/// Parameters for tools that require a wallet identifier.
#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct WalletIdParams {
    /// Wallet alias or 64-char hex seed hash
    pub wallet_id: String,
}

/// Parameters for tools that take no input.
#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct EmptyParams {}

/// Parameters for the `describe_tool` meta-tool.
#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct ToolNameParams {
    /// Tool name to describe
    pub name: String,
}
