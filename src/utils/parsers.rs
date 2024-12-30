//! Parsers for text input.

use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::platform::{DocumentQuery, DriveDocumentQuery};

pub(crate) trait TextInputParser {
    type Output;
    fn parse_input(&self, input: &str) -> Result<Self::Output, String>;
}

pub(crate) struct DocumentQueryTextInputParser {
    data_contract: DataContract,
}

impl DocumentQueryTextInputParser {
    pub(crate) fn new(data_contract: DataContract) -> Self {
        DocumentQueryTextInputParser { data_contract }
    }
}

impl TextInputParser for DocumentQueryTextInputParser {
    type Output = DocumentQuery;

    fn parse_input(&self, input: &str) -> Result<Self::Output, String> {
        DriveDocumentQuery::from_sql_expr(input, &self.data_contract, None)
            .map(Into::into)
            .map_err(|e| e.to_string())
    }
}
