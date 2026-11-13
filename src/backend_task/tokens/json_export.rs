//! Local export of the reviewed contract, independent of registration.

use super::{BackendTaskSuccessResult, TaskError};
use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tokio::{io::AsyncWriteExt, sync::Semaphore};

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
    path: PathBuf,
    json: String,
    replace: bool,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let opened = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(!replace)
        .truncate(replace)
        .open(&path)
        .await;
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if !replace && error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(BackendTaskSuccessResult::TokenContractJsonExists(path));
        }
        Err(source) => return Err(TaskError::TokenJsonSaveFailed { source }),
    };
    file.write_all(json.as_bytes())
        .await
        .map_err(|source| TaskError::TokenJsonSaveFailed { source })?;
    file.flush()
        .await
        .map_err(|source| TaskError::TokenJsonSaveFailed { source })?;
    Ok(BackendTaskSuccessResult::TokenContractJsonSaved(path))
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
            save(path.clone(), json.into(), false).await.unwrap(),
            BackendTaskSuccessResult::TokenContractJsonSaved(_)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), json.as_bytes());
        assert!(matches!(
            save(path.clone(), "replacement".into(), false)
                .await
                .unwrap(),
            BackendTaskSuccessResult::TokenContractJsonExists(_)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), json.as_bytes());
        save(path.clone(), "replacement".into(), true)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    }

    #[tokio::test]
    async fn token_json_export_reports_write_errors_with_the_original_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/contract.json");
        let error = save(path, "{}".into(), false).await.unwrap_err();
        assert!(
            matches!(&error, TaskError::TokenJsonSaveFailed { source } if source.kind() == std::io::ErrorKind::NotFound)
        );
        assert!(error.to_string().contains("Choose a writable folder"));
    }
}
