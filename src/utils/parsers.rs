//! Parsers for text input.

use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DocumentQuery, DriveDocumentQuery};

pub(crate) trait TextInputParser {
    type Output;
    fn parse_input(&self, input: &str) -> Result<Self::Output, String>;
}

pub(crate) struct DocumentQueryTextInputParser {
    data_contract: DataContract,
    /// Selects the where-clause grammar the input is parsed under, so the app
    /// accepts locally exactly what the network it talks to accepts.
    platform_version: &'static PlatformVersion,
}

impl DocumentQueryTextInputParser {
    pub(crate) fn new(
        data_contract: DataContract,
        platform_version: &'static PlatformVersion,
    ) -> Self {
        DocumentQueryTextInputParser {
            data_contract,
            platform_version,
        }
    }
}

impl TextInputParser for DocumentQueryTextInputParser {
    type Output = DocumentQuery;

    fn parse_input(&self, input: &str) -> Result<Self::Output, String> {
        DriveDocumentQuery::from_sql_expr(input, &self.data_contract, None, self.platform_version)
            .map(Into::into)
            .map_err(|e| e.to_string())
    }
}
