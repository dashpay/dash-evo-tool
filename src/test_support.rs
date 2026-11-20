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
