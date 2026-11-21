//! Local export of the reviewed contract, independent of registration.

use super::{BackendTaskSuccessResult, TaskError};
use std::{
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use tokio::sync::Semaphore;

static FILE_CHOOSER: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));

pub(super) async fn choose_path(path: PathBuf) -> Result<BackendTaskSuccessResult, TaskError> {
    let selected = single_chooser(FILE_CHOOSER.clone(), async move {
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title("Choose where to save contract JSON")
            .set_file_name(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("token-contract.json"),
            )
            .add_filter("JSON", &["json"]);
        if let Some(parent) = path.parent() {
            dialog = dialog.set_directory(parent);
        }
        dialog.save_file().await.map(|file| file.path().to_owned())
    })
    .await?;
    Ok(BackendTaskSuccessResult::TokenContractJsonPath(selected))
}

async fn single_chooser(
    gate: Arc<Semaphore>,
    choose: impl Future<Output = Option<PathBuf>> + Send + 'static,
) -> Result<Option<PathBuf>, TaskError> {
    let permit = gate
        .try_acquire_owned()
        .map_err(|_| TaskError::TokenJsonChooserBusy)?;
    // rfd's worker cannot be cancelled. Retain its permit even if the caller leaves.
    tokio::spawn(async move {
        let selected = choose.await;
        drop(permit);
        selected
    })
    .await
    .map_err(|source| TaskError::TokenJsonChooserFailed { source })
}

pub(super) async fn save(
    data_dir: PathBuf,
    path: PathBuf,
    json: String,
    replace: bool,
) -> Result<BackendTaskSuccessResult, TaskError> {
    tokio::task::spawn_blocking(move || save_file(&data_dir, path, &json, replace))
        .await
        .map_err(|source| TaskError::TokenJsonSaveWorkerFailed { source })?
}

fn check_destination(path: &Path, data_dir: &Path) -> Result<(), TaskError> {
    crate::utils::file_deletion::check_export_destination(path, data_dir).map_err(|source| {
        if source
            .get_ref()
            .is_some_and(|error| error.is::<crate::utils::file_deletion::DeletionRefused>())
        {
            TaskError::TokenJsonDestinationRefused { source }
        } else {
            TaskError::TokenJsonSaveFailed { source }
        }
    })
}

fn save_file(
    data_dir: &Path,
    path: PathBuf,
    json: &str,
    replace: bool,
) -> Result<BackendTaskSuccessResult, TaskError> {
    check_destination(&path, data_dir)?;
    if !replace
        && path
            .try_exists()
            .map_err(|source| TaskError::TokenJsonSaveFailed { source })?
    {
        return Ok(BackendTaskSuccessResult::TokenContractJsonExists(path));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|source| TaskError::TokenJsonSaveFailed { source })?;
    temporary
        .write_all(json.as_bytes())
        .map_err(|source| TaskError::TokenJsonSaveFailed { source })?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| TaskError::TokenJsonSaveFailed { source })?;
    check_destination(&path, data_dir)?;
    // Publish complete bytes in one rename; errors leave the existing destination intact.
    let persisted = if replace {
        temporary.persist(&path)
    } else {
        temporary.persist_noclobber(&path)
    };
    match persisted {
        Ok(_) => Ok(BackendTaskSuccessResult::TokenContractJsonSaved(path)),
        Err(error) if !replace && error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(BackendTaskSuccessResult::TokenContractJsonExists(path))
        }
        Err(error) => Err(TaskError::TokenJsonSaveFailed {
            source: error.error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_json_export_picker_remains_single_flight_after_caller_cancellation() {
        let gate = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("never-written.json");
        let selected = path.clone();
        let caller = tokio::spawn(single_chooser(gate.clone(), async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            Some(selected)
        }));
        started_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        for _ in 0..3 {
            assert!(matches!(
                single_chooser(gate.clone(), async {
                    panic!("Second native chooser started")
                })
                .await,
                Err(TaskError::TokenJsonChooserBusy)
            ));
        }
        finish_tx.send(()).unwrap();
        drop(
            tokio::time::timeout(std::time::Duration::from_secs(2), gate.acquire())
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(
            single_chooser(gate, async { None })
                .await
                .unwrap()
                .is_none()
        );
        assert!(!path.exists(), "Selecting a path must never write a file");
    }

    #[tokio::test]
    async fn token_json_export_preserves_bytes_and_requires_explicit_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contract.json");
        let json = "{\n  \"name\": \"Zażółć 🪙\",\n  \"amount\": 18446744073709551615\n}";
        assert!(matches!(
            save(dir.path().to_owned(), path.clone(), json.into(), false)
                .await
                .unwrap(),
            BackendTaskSuccessResult::TokenContractJsonSaved(_)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), json.as_bytes());
        assert!(matches!(
            save(
                dir.path().to_owned(),
                path.clone(),
                "replacement".into(),
                false
            )
            .await
            .unwrap(),
            BackendTaskSuccessResult::TokenContractJsonExists(_)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), json.as_bytes());
        save(
            dir.path().to_owned(),
            path.clone(),
            "replacement".into(),
            true,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn token_json_export_refuses_symlinks_without_changing_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sentinel.txt");
        let link = dir.path().join("contract.json");
        std::fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let result = save(dir.path().to_owned(), link, "replacement".into(), true).await;
        assert!(result.is_err(), "An export must refuse a symbolic link");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
    }

    #[tokio::test]
    async fn token_json_export_refuses_protected_application_filenames() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            ".env",
            "det-app.sqlite",
            "data.db",
            "det-testnet.sqlite-wal",
            "secrets.pwsvault",
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, "original").unwrap();
            assert!(
                save(
                    dir.path().to_owned(),
                    path.clone(),
                    "replacement".into(),
                    true
                )
                .await
                .is_err(),
                "Protected destination: {name}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn token_json_export_keeps_existing_bytes_when_replacement_cannot_be_committed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contract.json");
        std::fs::write(&path, "original").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = save(
            dir.path().to_owned(),
            path.clone(),
            "replacement".into(),
            true,
        )
        .await;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            result.is_err(),
            "Replacing a file requires a writable parent directory"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }

    #[tokio::test]
    async fn token_json_export_reports_write_errors_with_the_original_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/contract.json");
        let error = save(dir.path().to_owned(), path, "{}".into(), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, TaskError::TokenJsonSaveFailed { source } if source.kind() == std::io::ErrorKind::NotFound)
        );
        assert!(error.to_string().contains("Choose a writable folder"));
    }
}
