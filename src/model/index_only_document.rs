//! `indexOnly` document types (protocol version 14).
//!
//! An indexOnly document type keeps no stored document: Platform keeps only
//! its index entries, and a query rebuilds each row from them. Such rows are
//! read-only — the type can never be replaced, transferred or traded — and a
//! delete names the entry by the values it was created with rather than by
//! id. The id a fetched row carries is computed from its index position and
//! addresses nothing on chain; a delete may carry it, since the entries are
//! matched by values alone.
//!
//! These checks mirror what the SDK refuses before signing
//! (`index_only_delete_document`), with typed reasons instead of its text.

use std::collections::BTreeMap;

use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::data_contract::document_type::accessors::{
    DocumentTypeV0Getters, DocumentTypeV2Getters,
};
use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeV0Methods;
use dash_sdk::dpp::data_contract::document_type::{
    DocumentTypeRef, index_only_row_commits_created_at, missing_required_value_path,
    undeclared_value_path,
};
use dash_sdk::dpp::document::property_names::CREATED_AT;
use dash_sdk::dpp::document::{Document, DocumentV0Getters};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::dpp::version::PlatformVersion;
use thiserror::Error;

/// Whether `document_type` keeps only index entries (an indexOnly type).
pub fn is_index_only(document_type: DocumentTypeRef) -> bool {
    document_type.index_only()
}

/// Whether `platform_version` defines the transition that deletes an
/// indexOnly document. Upstream names no feature constant; the transition's
/// serialization bounds exist exactly from the version that accepts it.
pub fn index_only_delete_supported(platform_version: &PlatformVersion) -> bool {
    platform_version
        .dpp
        .state_transition_serialization_versions
        .document_index_only_delete_state_transition
        .is_some()
}

/// Why an indexOnly entry cannot be deleted with the given values.
#[derive(Debug, Error)]
pub enum IndexOnlyDeleteError {
    #[error(
        "This document type keeps full documents, so its documents are deleted by ID. Enter the document ID instead."
    )]
    NotIndexOnly,
    #[error("Entries of this document type cannot be deleted.")]
    NotDeletable,
    #[error(
        "The values name a field this document type does not have: {path}. Remove it and try again."
    )]
    UndeclaredValue { path: String },
    #[error(
        "The values are missing the field {path}. Add the value the entry was created with and try again."
    )]
    MissingValue { path: String },
    #[error(
        "The values are missing the creation time ($createdAt). Add the creation time the entry was created with and try again."
    )]
    MissingCreatedAt,
    #[error(
        "The creation time ($createdAt) must be a whole number of milliseconds. Correct it and try again."
    )]
    InvalidCreatedAt,
    #[error("The values are not valid JSON. Correct them and try again.")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("The values must be a JSON object keyed by field name. Correct them and try again.")]
    NotAnObject {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "The values could not be checked against this document type. Select the document type again and retry."
    )]
    Schema {
        #[source]
        source: Box<ProtocolError>,
    },
}

/// The values an indexOnly delete is built from: the entry's own properties
/// and, kept apart as the SDK takes it, its `$createdAt`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexOnlyDeleteValues {
    pub values: BTreeMap<String, Value>,
    pub created_at: Option<TimestampMillis>,
}

impl IndexOnlyDeleteValues {
    /// The values a fetched (rebuilt) row carries. A row fetched through an
    /// index that covers only some properties lacks the others, so these
    /// are a starting point the user may need to complete.
    pub fn from_document(document: &Document) -> Self {
        Self {
            values: document.properties().clone(),
            created_at: document.created_at(),
        }
    }

    /// The values as editable JSON, `$createdAt` included when known.
    pub fn to_json(&self) -> String {
        let mut object = serde_json::Map::new();
        for (name, value) in &self.values {
            // A platform value always has a JSON form; one that could not be
            // shown is left out, and the check reports it if it is required.
            if let Ok(json) = serde_json::to_value(value) {
                object.insert(name.clone(), json);
            }
        }
        if let Some(created_at) = self.created_at {
            object.insert(CREATED_AT.to_string(), created_at.into());
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(object)).unwrap_or_default()
    }

    /// Parses values edited as JSON, splitting `$createdAt` off. Identifiers
    /// and byte arrays may stay in their text form: the delete converts them
    /// with the document type's schema, like a create does.
    pub fn from_json(json: &str) -> Result<Self, IndexOnlyDeleteError> {
        let parsed: serde_json::Value = serde_json::from_str(json)
            .map_err(|source| IndexOnlyDeleteError::InvalidJson { source })?;
        let mut values: BTreeMap<String, Value> = serde_json::from_value(parsed)
            .map_err(|source| IndexOnlyDeleteError::NotAnObject { source })?;
        let created_at = values
            .remove(CREATED_AT)
            .map(|value| {
                value
                    .to_integer::<TimestampMillis>()
                    .map_err(|_| IndexOnlyDeleteError::InvalidCreatedAt)
            })
            .transpose()?;
        Ok(Self { values, created_at })
    }
}

/// Refuses, before anything is signed, a delete of an indexOnly entry that
/// Platform would refuse after the fee is paid: the same checks, in the same
/// order, as the SDK's `index_only_delete_document`. A value of the wrong
/// shape, or one that differs from the original entry, is only found by
/// Platform.
// TODO(upstream): the SDK reports these refusals as `Error::Generic` text
// (and the platform-wallet wrapper as `InvalidIdentityData` text), so DET
// repeats them here to get typed reasons. Drop this once the SDK returns a
// typed error for them.
pub fn validate_index_only_delete(
    document_type: DocumentTypeRef,
    delete: &IndexOnlyDeleteValues,
    platform_version: &PlatformVersion,
) -> Result<(), IndexOnlyDeleteError> {
    if !is_index_only(document_type) {
        return Err(IndexOnlyDeleteError::NotIndexOnly);
    }
    if !document_type.documents_can_be_deleted() {
        return Err(IndexOnlyDeleteError::NotDeletable);
    }
    let mut values = delete.values.clone();
    document_type.sanitize_document_properties(&mut values);
    if let Some(path) = undeclared_value_path(document_type, &values) {
        return Err(IndexOnlyDeleteError::UndeclaredValue { path });
    }
    if delete.created_at.is_none()
        && index_only_row_commits_created_at(
            document_type.required_fields(),
            document_type.index_structure(),
        )
    {
        return Err(IndexOnlyDeleteError::MissingCreatedAt);
    }
    if let Some(path) = missing_required_value_path(document_type, &values, platform_version)
        .map_err(|source| IndexOnlyDeleteError::Schema {
            source: Box::new(source),
        })?
    {
        return Err(IndexOnlyDeleteError::MissingValue { path });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use dash_sdk::dpp::data_contract::DataContract;
    use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
    use dash_sdk::dpp::version::PlatformVersion;

    /// A protocol 14 contract with an indexOnly `like` type (modelled on
    /// upstream's yappr-likes fixture), a `stamp` indexOnly type whose
    /// entries commit to `$createdAt`, and a stored `note` type.
    pub(crate) fn index_only_contract() -> DataContract {
        let json = serde_json::json!({
            "$formatVersion": "1",
            "id": "AY6xWncZUFv2GCrS5seqKthUfbW9yYyUXtF8diSuHQ4g",
            "ownerId": "AtirhSVpAWF7dEt6dLAmesC4Sr1MsJ9bFC1nLAoNnq2S",
            "version": 1,
            "documentSchemas": {
                "like": {
                    "type": "object",
                    "indexOnly": true,
                    "documentsMutable": false,
                    "canBeDeleted": true,
                    "indices": [
                        {
                            "name": "byLiker",
                            "properties": [{"$ownerId": "asc"}],
                            "terminal": "postId"
                        }
                    ],
                    "properties": {
                        "postId": {
                            "type": "array",
                            "byteArray": true,
                            "minItems": 32,
                            "maxItems": 32,
                            "contentMediaType": "application/x.dash.dpp.identifier",
                            "position": 0
                        }
                    },
                    "required": ["postId"],
                    "additionalProperties": false
                },
                "stamp": {
                    "type": "object",
                    "indexOnly": true,
                    "documentsMutable": false,
                    "canBeDeleted": true,
                    "indices": [
                        {
                            "name": "byOwner",
                            "properties": [{"$ownerId": "asc"}],
                            "terminal": "label"
                        },
                        {
                            "name": "byOwnerTime",
                            "properties": [{"$ownerId": "asc"}, {"$createdAt": "asc"}],
                            "terminal": "label"
                        }
                    ],
                    "properties": {
                        "label": {"type": "string", "maxLength": 20, "position": 0}
                    },
                    "required": ["label", "$createdAt"],
                    "additionalProperties": false
                },
                "badge": {
                    "type": "object",
                    "indexOnly": true,
                    "documentsMutable": false,
                    "canBeDeleted": true,
                    "indices": [
                        {
                            "name": "byOwner",
                            "properties": [{"$ownerId": "asc"}],
                            "terminal": "code"
                        }
                    ],
                    "properties": {
                        "code": {
                            "type": "array",
                            "byteArray": true,
                            "minItems": 6,
                            "maxItems": 6,
                            "position": 0
                        }
                    },
                    "required": ["code"],
                    "additionalProperties": false
                },
                "note": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "maxLength": 20, "position": 0}
                    },
                    "additionalProperties": false
                }
            }
        });
        DataContract::from_json(json, true, PlatformVersion::get(14).expect("protocol 14"))
            .expect("the indexOnly fixture contract parses")
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::index_only_contract;
    use super::*;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::document::DocumentV0;
    use dash_sdk::dpp::platform_value::Identifier;

    fn pv14() -> &'static PlatformVersion {
        PlatformVersion::get(14).expect("protocol 14")
    }

    fn like_values(post: [u8; 32]) -> IndexOnlyDeleteValues {
        IndexOnlyDeleteValues {
            values: BTreeMap::from([("postId".to_string(), Value::Identifier(post))]),
            created_at: None,
        }
    }

    #[test]
    fn index_only_types_are_recognised() {
        let contract = index_only_contract();
        assert!(is_index_only(
            contract.document_type_for_name("like").unwrap()
        ));
        assert!(!is_index_only(
            contract.document_type_for_name("note").unwrap()
        ));
    }

    #[test]
    fn the_index_only_delete_exists_from_protocol_14() {
        assert!(!index_only_delete_supported(
            PlatformVersion::get(13).expect("protocol 13")
        ));
        assert!(index_only_delete_supported(pv14()));
    }

    #[test]
    fn complete_values_pass() {
        let contract = index_only_contract();
        let like = contract.document_type_for_name("like").unwrap();
        validate_index_only_delete(like, &like_values([7; 32]), pv14()).expect("valid delete");
    }

    #[test]
    fn a_stored_document_type_is_refused() {
        let contract = index_only_contract();
        let note = contract.document_type_for_name("note").unwrap();
        assert!(matches!(
            validate_index_only_delete(note, &IndexOnlyDeleteValues::default(), pv14()),
            Err(IndexOnlyDeleteError::NotIndexOnly)
        ));
    }

    #[test]
    fn an_undeclared_value_is_refused() {
        let contract = index_only_contract();
        let like = contract.document_type_for_name("like").unwrap();
        let mut delete = like_values([7; 32]);
        delete
            .values
            .insert("comment".to_string(), Value::Text("hi".to_string()));
        assert!(matches!(
            validate_index_only_delete(like, &delete, pv14()),
            Err(IndexOnlyDeleteError::UndeclaredValue { ref path }) if path == "comment"
        ));
    }

    #[test]
    fn a_missing_required_value_is_refused() {
        let contract = index_only_contract();
        let like = contract.document_type_for_name("like").unwrap();
        assert!(matches!(
            validate_index_only_delete(like, &IndexOnlyDeleteValues::default(), pv14()),
            Err(IndexOnlyDeleteError::MissingValue { ref path }) if path == "postId"
        ));
    }

    #[test]
    fn a_missing_creation_time_is_refused_where_the_entry_commits_to_it() {
        let contract = index_only_contract();
        let stamp = contract.document_type_for_name("stamp").unwrap();
        let mut delete = IndexOnlyDeleteValues {
            values: BTreeMap::from([("label".to_string(), Value::Text("a".to_string()))]),
            created_at: None,
        };
        assert!(matches!(
            validate_index_only_delete(stamp, &delete, pv14()),
            Err(IndexOnlyDeleteError::MissingCreatedAt)
        ));
        delete.created_at = Some(1_700_000_000_000);
        validate_index_only_delete(stamp, &delete, pv14()).expect("valid with the time");
    }

    /// A row's values survive the JSON the user edits: identifiers come back
    /// as their text form and the schema turns them back into identifiers.
    #[test]
    fn a_rows_values_round_trip_through_json() {
        let contract = index_only_contract();
        let like = contract.document_type_for_name("like").unwrap();
        let document: Document = DocumentV0 {
            id: Identifier::new([1; 32]),
            owner_id: Identifier::new([2; 32]),
            properties: like_values([7; 32]).values,
            created_at: Some(1_700_000_000_000),
            ..Default::default()
        }
        .into();
        let from_row = IndexOnlyDeleteValues::from_document(&document);
        assert_eq!(from_row.created_at, Some(1_700_000_000_000));

        let parsed = IndexOnlyDeleteValues::from_json(&from_row.to_json()).expect("parses");
        assert_eq!(parsed.created_at, Some(1_700_000_000_000));
        let mut sanitized = parsed.values.clone();
        like.sanitize_document_properties(&mut sanitized);
        assert_eq!(
            sanitized.get("postId").and_then(|v| v.to_identifier().ok()),
            Some(Identifier::new([7; 32]))
        );
        validate_index_only_delete(like, &parsed, pv14()).expect("still a valid delete");
    }

    /// Bytes whose base64 text ("abcdabcd") is also valid hex: the schema
    /// reads text as hex first, so the JSON must not show bytes as base64.
    #[test]
    fn byte_array_values_round_trip_through_json() {
        let contract = index_only_contract();
        let badge = contract.document_type_for_name("badge").unwrap();
        let code = vec![0x69, 0xb7, 0x1d, 0x69, 0xb7, 0x1d];
        let delete = IndexOnlyDeleteValues {
            values: BTreeMap::from([("code".to_string(), Value::Bytes(code.clone()))]),
            created_at: None,
        };
        let parsed = IndexOnlyDeleteValues::from_json(&delete.to_json()).expect("parses");
        let mut sanitized = parsed.values;
        badge.sanitize_document_properties(&mut sanitized);
        assert_eq!(
            sanitized.get("code").and_then(|v| v.to_binary_bytes().ok()),
            Some(code)
        );
    }

    #[test]
    fn malformed_json_is_refused() {
        assert!(matches!(
            IndexOnlyDeleteValues::from_json("{"),
            Err(IndexOnlyDeleteError::InvalidJson { .. })
        ));
        assert!(matches!(
            IndexOnlyDeleteValues::from_json("[1, 2]"),
            Err(IndexOnlyDeleteError::NotAnObject { .. })
        ));
        assert!(matches!(
            IndexOnlyDeleteValues::from_json(r#"{"$createdAt": "soon"}"#),
            Err(IndexOnlyDeleteError::InvalidCreatedAt)
        ));
    }
}
