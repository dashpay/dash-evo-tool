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

/// Switch off kittest's check that every input widget has an accessible name,
/// for a harness whose screen still has unnamed widgets. Every unit-test
/// opt-out goes through here so the remaining gaps stay in one list.
//
// TODO: name these widgets for screen readers, then drop the opt-out for their
// screens: the amount field (`AmountInput`); the top-up form's wallet selector
// and amount field; the funding wallet picker (Add New Identity); the wallet
// picker and identity fields (Add Existing Identity); the add-key source combo
// box; the identity send form's radio buttons; the document action form's
// combo boxes and text fields; the transition visualizer input; the DPNS
// username field. Integration harnesses list theirs in `tests/kittest/support.rs`.
#[cfg(test)]
pub(crate) fn allowing_unnamed_widgets<State>(
    builder: egui_kittest::HarnessBuilder<State>,
) -> egui_kittest::HarnessBuilder<State> {
    builder.with_accessibility_check(false)
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
        // "disabled" after this capture registered. A global subscriber that
        // never rules a callsite or level out keeps those caches open, and the
        // thread's own subscriber decides per event.
        static KEEP_CALLSITES_OPEN: std::sync::Once = std::sync::Once::new();
        KEEP_CALLSITES_OPEN.call_once(|| {
            let _ = tracing::subscriber::set_global_default(KeepCallsitesOpen);
        });
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
