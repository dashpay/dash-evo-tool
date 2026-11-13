//! Screen-owned JSON snapshot and file chooser request ownership.

use crate::{
    app::AppAction,
    backend_task::{
        BackendTask, BackendTaskContext, BackendTaskSuccessResult, error::TaskError,
        tokens::TokenTask,
    },
};
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;

/// An export keeps the reviewed bytes until the user saves or dismisses it.
pub(crate) struct ContractJsonExport {
    json: String,
    pub path: String,
    native_path: Option<PathBuf>,
    pub picker: Option<BackendTaskContext>,
    pub saving: Option<BackendTaskContext>,
    pub replacement: Option<PathBuf>,
    pub notice: Option<String>,
}

impl ContractJsonExport {
    /// Captures reviewed bytes without reading the filesystem during rendering.
    pub fn new(json: String) -> Self {
        Self {
            json,
            path: String::new(),
            native_path: None,
            picker: None,
            saving: None,
            replacement: None,
            notice: None,
        }
    }

    fn file_path(&self) -> PathBuf {
        self.native_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(&self.path))
    }

    /// Invalidates native path bytes and replacement approval after a text edit.
    pub fn path_edited(&mut self) {
        self.native_path = None;
        self.replacement = None;
        self.notice = None;
    }

    /// Dispatches one chooser request owned by this export.
    pub fn browse(&mut self, network: Network) -> AppAction {
        if self.picker.is_some() || self.saving.is_some() {
            return AppAction::None;
        }
        let task = BackendTask::TokenTask(Box::new(TokenTask::ChooseTokenContractJsonPath(
            self.file_path(),
        )));
        let context = BackendTaskContext::for_dispatch_on(&task, network);
        self.picker = Some(context.clone());
        self.replacement = None;
        self.notice = None;
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Discards chooser interest without allowing a late reply to change the path.
    pub fn cancel_picker(&mut self) {
        self.picker = None;
        self.notice = Some(
            "You can enter a file path and save. Close the file chooser separately if it appears."
                .into(),
        );
    }

    /// Dispatches a confirmed save, binding replacement approval to its path.
    pub fn save(&mut self, network: Network, replace: bool) -> AppAction {
        let path = self.file_path();
        if self.picker.is_some()
            || self.saving.is_some()
            || self.path.is_empty()
            || (replace && self.replacement.as_ref() != Some(&path))
        {
            return AppAction::None;
        }
        let task = BackendTask::TokenTask(Box::new(TokenTask::SaveTokenContractJson {
            path,
            json: self.json.clone(),
            replace,
        }));
        let context = BackendTaskContext::for_dispatch_on(&task, network);
        self.saving = Some(context.clone());
        self.replacement = None;
        self.notice = None;
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Returns true only when this export's explicit save has completed.
    pub fn result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> bool {
        if self.picker.as_ref() == Some(context)
            && let BackendTaskSuccessResult::TokenContractJsonPath(path) = result
        {
            self.picker = None;
            if let Some(path) = path {
                self.path = path.to_string_lossy().into_owned();
                self.native_path = Some(path.clone());
                self.replacement = None;
            } else {
                self.notice =
                    Some("No file was chosen. Enter a file path or try Browse again.".into());
            }
        }
        if self.saving.as_ref() == Some(context) {
            match result {
                BackendTaskSuccessResult::TokenContractJsonSaved(_) => return true,
                BackendTaskSuccessResult::TokenContractJsonExists(path) => {
                    self.saving = None;
                    self.replacement = Some(path.clone());
                }
                _ => {}
            }
        }
        false
    }

    /// Keeps the snapshot and destination available for retry after a matching failure.
    pub fn error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        if self.picker.as_ref() == Some(context) || self.saving.as_ref() == Some(context) {
            self.picker = None;
            self.saving = None;
            self.notice = Some(error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: AppAction) -> (TokenTask, BackendTaskContext) {
        let AppAction::BackendTaskWithContext {
            task: BackendTask::TokenTask(task),
            context,
        } = action
        else {
            panic!("Expected an export request")
        };
        (*task, context)
    }

    #[test]
    fn token_json_export_requires_a_destination_before_dispatching_a_write() {
        let mut export = ContractJsonExport::new("snapshot".into());
        assert!(export.path.is_empty());
        assert!(matches!(
            export.save(Network::Testnet, false),
            AppAction::None
        ));
    }

    #[test]
    fn token_json_export_cancel_reopen_and_late_picker_result_never_write() {
        let mut export = ContractJsonExport::new("reviewed bytes".into());
        let (_, old_request) = request(export.browse(Network::Testnet));
        assert!(matches!(export.browse(Network::Testnet), AppAction::None));
        export.cancel_picker();
        export.path = "/manual/path.json".into();
        export.path_edited();
        assert!(!export.result(
            &old_request,
            &BackendTaskSuccessResult::TokenContractJsonPath(Some("/late/path.json".into()))
        ));
        assert_eq!(export.file_path(), PathBuf::from("/manual/path.json"));
        assert!(export.saving.is_none());
        let mut reopened = ContractJsonExport::new("new snapshot".into());
        let (_, current) = request(reopened.browse(Network::Testnet));
        let before = reopened.file_path();
        reopened.result(
            &old_request,
            &BackendTaskSuccessResult::TokenContractJsonPath(Some("/late/path.json".into())),
        );
        assert_eq!(reopened.file_path(), before);
        assert_eq!(reopened.picker, Some(current));
        let (task, _) = request(export.save(Network::Testnet, false));
        assert!(
            matches!(task, TokenTask::SaveTokenContractJson { json, .. } if json == "reviewed bytes")
        );
        assert!(matches!(
            export.save(Network::Testnet, false),
            AppAction::None
        ));
    }

    #[test]
    fn token_json_export_path_changes_require_fresh_replacement_confirmation() {
        let mut export = ContractJsonExport::new("snapshot".into());
        export.path = "contract.json".into();
        let (_, pending) = request(export.save(Network::Testnet, false));
        let first = export.file_path();
        export.result(
            &pending,
            &BackendTaskSuccessResult::TokenContractJsonExists(first),
        );
        export.path = "/different/path.json".into();
        export.path_edited();
        assert!(matches!(
            export.save(Network::Testnet, true),
            AppAction::None
        ));
        let (_, pending) = request(export.save(Network::Testnet, false));
        export.result(
            &pending,
            &BackendTaskSuccessResult::TokenContractJsonExists(export.file_path()),
        );
        let (task, _) = request(export.save(Network::Testnet, true));
        assert!(
            matches!(task, TokenTask::SaveTokenContractJson { path, replace: true, .. } if path == PathBuf::from("/different/path.json"))
        );
    }

    #[test]
    fn token_json_export_picker_and_write_errors_preserve_snapshot_and_path() {
        let mut export = ContractJsonExport::new("snapshot".into());
        export.path = "contract.json".into();
        let path = export.file_path();
        let (_, pending) = request(export.browse(Network::Testnet));
        export.error(&pending, &TaskError::TokenJsonChooserBusy);
        assert_eq!(export.file_path(), path);
        let (_, pending) = request(export.save(Network::Testnet, false));
        export.error(
            &pending,
            &TaskError::TokenJsonSaveFailed {
                source: std::io::ErrorKind::PermissionDenied.into(),
            },
        );
        assert_eq!(export.file_path(), path);
        let (task, _) = request(export.save(Network::Testnet, false));
        assert!(
            matches!(task, TokenTask::SaveTokenContractJson { json, .. } if json == "snapshot")
        );
    }

    #[cfg(unix)]
    #[test]
    fn token_json_export_preserves_native_path_bytes_until_edited() {
        use std::os::unix::ffi::OsStringExt;
        let mut export = ContractJsonExport::new("{}".into());
        let (_, pending) = request(export.browse(Network::Testnet));
        let path = PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/contract-\xff.json".to_vec(),
        ));
        export.result(
            &pending,
            &BackendTaskSuccessResult::TokenContractJsonPath(Some(path.clone())),
        );
        let (task, _) = request(export.save(Network::Testnet, false));
        assert!(
            matches!(task, TokenTask::SaveTokenContractJson { path: saved, .. } if saved == path)
        );
    }
}
