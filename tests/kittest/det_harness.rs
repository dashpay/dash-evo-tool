use dash_evo_tool::app::AppState;
use egui_kittest::Harness;

/// Test helper and tools for running egui tests of the Dash Evo Tool app
pub struct DETHarness<'a> {
    pub kittest: Harness<'a, AppState>,
    pub name: String,
}

impl DETHarness<'_> {
    /// Create a new test harness for the Dash Evo Tool app.
    ///
    /// `name` is used to identify the test and will be part of the snapshot file name.
    pub fn new(name: &str) -> Self {
        Self::setup_logging();

        let harness = egui_kittest::Harness::builder()
            .with_max_steps(100)
            .build_eframe(|ctx| AppState::new(ctx.egui_ctx.clone()).with_animations(false));

        let mut me = DETHarness {
            kittest: harness,
            name: name.to_string(),
        };

        // Set the window size for the test
        me.kittest.set_size(egui::vec2(800.0, 600.0));
        // Run one frame to ensure the app initializes
        me.kittest.run();

        me
    }

    fn setup_logging() {
        tracing_subscriber::fmt()
            .with_env_filter("error, dash_evo_tool=debug,kittest=trace")
            .init();
    }

    pub fn state(&self) -> &egui_kittest::kittest::State {
        self.kittest.kittest_state()
    }

    /// Execute a potentially panicking operation and continue execution
    /// Takes a snapshot on panic and returns whether the operation succeeded
    pub fn try_execute<F, R>(
        &mut self,
        operation_name: &str,
        operation: F,
    ) -> Result<R, Box<dyn std::any::Any + Send>>
    where
        F: FnOnce(&mut Self) -> R + std::panic::UnwindSafe,
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(5)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        let result = runtime.block_on(self.execute(operation_name, operation));
        runtime.shutdown_background();

        // let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));

        result
    }

    async fn execute<F, R>(
        &mut self,
        operation_name: &str,
        operation: F,
    ) -> Result<R, Box<dyn std::any::Any + Send>>
    where
        F: FnOnce(&mut Self) -> R + std::panic::UnwindSafe,
    {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));

        match result {
            Ok(value) => Ok(value),
            Err(panic_info) => {
                eprintln!(
                    "Operation '{}' panicked, taking snapshot...",
                    operation_name
                );
                self.kittest
                    .snapshot(&format!("{}_{}_panic", self.name, operation_name));
                Err(panic_info)
            }
        }
    }

    /// Run the test harness, executing all registered operations.
    ///
    /// See [Harness::run] for more details.
    pub fn run(&mut self) {
        self.kittest.run();
    }
}
