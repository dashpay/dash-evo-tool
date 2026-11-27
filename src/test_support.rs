//! Shared support for tests across the library crate.
//!
//! Compiled only for unit tests and the `testing` / `bench` features, never
//! into release builds. [`keys`] is public so integration tests and benches
//! can reach it; everything else is unit-test only.

pub mod keys;

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
use dash_sdk::dpp::consensus::ConsensusError;
#[cfg(test)]
use dash_sdk::dpp::consensus::state::document::duplicate_unique_index_error::DuplicateUniqueIndexError;
#[cfg(test)]
use dash_sdk::platform::Identifier;

/// A kittest harness builder that installs the app's bundled fonts before the
/// first frame, so a screen mounted without `AppState` draws every glyph the app can.
#[cfg(test)]
pub(crate) fn harness_builder_with_app_fonts<State>() -> egui_kittest::HarnessBuilder<State> {
    egui_kittest::Harness::builder().with_context_setup(|ctx| {
        ctx.set_fonts(crate::bundled::fonts().expect("bundled fonts load"));
    })
}

#[cfg(test)]
const CHILD_MARKER: &str = "DET_ISOLATED_TEST_CHILD";

/// Whether file modes keep this process out. Not for one that bypasses them
/// (root, or `CAP_DAC_OVERRIDE` in a container): a test that denies access
/// through a mode cannot fail the way it means to there, so it returns early.
#[cfg(all(test, unix))]
pub(crate) fn file_modes_deny_access() -> bool {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500))
        .expect("deny writes");
    let denied = std::fs::write(dir.path().join("probe"), b"").is_err();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).expect("restore");
    denied
}

/// Whether this process is a child started by [`run_test_in_child`].
#[cfg(test)]
pub(crate) fn is_test_child() -> bool {
    std::env::var_os(CHILD_MARKER).is_some()
}

/// Runs the unit test `test` (full path) alone in a child process with
/// `envs` set and no inherited network settings, and asserts it passed.
/// Returns `false` inside the child, which then runs the test body.
#[cfg(test)]
pub(crate) fn run_test_in_child(test: &str, envs: &[(&str, &std::path::Path)]) -> bool {
    if is_test_child() {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().expect("test binary"));
    command.args([test, "--exact"]).env(CHILD_MARKER, "1");
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if ["MAINNET_", "TESTNET_", "DEVNET_", "LOCAL_"]
            .iter()
            .any(|prefix| key.starts_with(prefix))
        {
            command.env_remove(key.as_ref());
        }
    }
    command.envs(envs.iter().copied());
    let output = command.output().expect("run isolated test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated test failed: {stdout}\n{stderr}"
    );
    assert!(stdout.contains("test result: ok. 1 passed;"), "{stdout}");
    true
}

/// Serializes every unit test that mutates the process-global data directory.
#[cfg(test)]
pub(crate) static DASH_EVO_DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn duplicate_unique_index_broadcast_error(properties: Vec<&str>) -> dash_sdk::Error {
    let consensus = ConsensusError::from(DuplicateUniqueIndexError::new(
        Identifier::random(),
        properties.into_iter().map(str::to_string).collect(),
    ));

    dash_sdk::Error::StateTransitionBroadcastError(dash_sdk::error::StateTransitionBroadcastError {
        code: 40105,
        message: "duplicate unique index".to_string(),
        cause: Some(consensus),
    })
}

/// Text written by `tracing` on this thread while the returned guard lives.
#[cfg(test)]
pub(crate) struct CapturedLog {
    text: std::sync::Arc<Mutex<Vec<u8>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

#[cfg(test)]
impl CapturedLog {
    /// Capture every event on this thread, any level, without ANSI colours.
    pub(crate) fn start() -> Self {
        // tracing-core recomputes its callsite and max-level caches whenever a
        // thread's subscriber comes or goes, from whichever dispatchers are
        // alive then; a concurrent test dropping its subscriber can store
        // "disabled" after this capture registered. A dispatcher that never
        // rules a callsite or level out, kept alive here, is counted in every
        // recompute (`Dispatch::new` registers it) without becoming anyone's
        // default; the thread's own subscriber decides per event.
        static KEEP_CALLSITES_OPEN: std::sync::OnceLock<tracing::Dispatch> =
            std::sync::OnceLock::new();
        KEEP_CALLSITES_OPEN.get_or_init(|| tracing::Dispatch::new(KeepCallsitesOpen));
        let text = std::sync::Arc::new(Mutex::new(Vec::new()));
        let writer = std::sync::Arc::clone(&text);
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || CapturedLogSink(std::sync::Arc::clone(&writer)))
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        Self {
            text,
            _guard: tracing::subscriber::set_default(subscriber),
        }
    }

    /// Everything captured so far.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.text.lock().expect("captured log")).into_owned()
    }
}

/// Records nothing, but never lets tracing cache a callsite or level as off.
#[cfg(test)]
struct KeepCallsitesOpen;

#[cfg(test)]
impl tracing::Subscriber for KeepCallsitesOpen {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[cfg(test)]
struct CapturedLogSink(std::sync::Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl std::io::Write for CapturedLogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("captured log").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture stays on its own thread: it never installs a process-wide
    /// default, which would keep the app's own logger from installing.
    #[test]
    fn captured_log_installs_no_process_wide_subscriber() {
        let _log = CapturedLog::start();
        let elsewhere = std::thread::spawn(|| {
            tracing::dispatcher::get_default(|dispatch| dispatch.is::<KeepCallsitesOpen>())
        })
        .join()
        .expect("probe thread");
        assert!(!elsewhere, "the capture's keeper became the global default");
    }
}
