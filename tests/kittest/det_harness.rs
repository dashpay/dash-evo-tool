use dash_evo_tool::app::AppState;
use dash_sdk::dpp::dashcore::Network;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{Node, Queryable},
    Harness,
};

/// Test helper and tools for running egui tests of the Dash Evo Tool app
pub struct DETHarness<'a> {
    pub kittest: Harness<'a, AppState>,
    name: String,
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
        // Fixme: find out how to scroll the window
        me.kittest.set_size(egui::vec2(800.0, 3000.0));
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

    /// Connect to a selected network
    pub fn connect_to_network(&mut self, network: Network) {
        let index = match network {
            Network::Dash => 0,
            Network::Testnet => 1,
            Network::Devnet => 2,
            Network::Regtest => 3,
            _ => panic!("Unsupported network"),
        };
        self.click_by_label("N");
        self.run();

        self.click_by_label(&format!("select_network_{}", network.magic()));

        // start dash-qt
        let start = self
            .kittest
            .kittest_state()
            .get_all_by_role_and_label(Role::Button, "Start")
            .nth(index)
            .expect("Button for the network not found");
        start.click();
        self.run();

        // We need to wait for dash-qt to start and sync
        std::thread::sleep(std::time::Duration::from_secs(6));

        self.snapshot("network_connected");
    }
}

impl<'a> DETHarness<'a> {
    /// Run the test harness, executing all registered operations.
    ///
    /// See [Harness::run] for more details.
    pub fn run(&mut self) {
        self.kittest.run();
    }
    /// Click a button by label
    pub fn click_by_label(&mut self, label: &str) {
        let btn = self.kittest.kittest_state().get_by_label(label);
        btn.click();
        self.run();
    }

    /// Click a button by value
    pub fn click_by_value(&mut self, value: &str) {
        let btn = self.kittest.kittest_state().get_by_value(value);
        btn.click();
        self.run();
    }

    /// Set text in a field by label
    pub fn set_text_by_label(&mut self, label: &str, text: &str) {
        let field = self.kittest.kittest_state().get_by_label(label);
        field.type_text(text);
        self.run();
    }

    /// Get a node by label
    ///
    /// ## Panics
    ///
    /// All get_ functions will panic if the node is not found.
    pub fn get_by_label(&'a self, button: &'a str) -> Node<'a> {
        self.kittest.kittest_state().get_by_label(button)
    }

    /// Query all nodes by label
    pub fn query_all_by_label(&'a self, label: &'a str) -> Vec<egui_kittest::kittest::Node<'a>> {
        self.kittest
            .kittest_state()
            .query_all_by_label(label)
            .collect()
    }

    /// Take a snapshot
    pub fn snapshot(&mut self, name: &str) {
        self.kittest.snapshot(name);
    }
    /// Wait until label is present or timeout occurs
    pub fn wait_all_by_label<'b>(
        &'b mut self,
        label: &'a str,
        timeout: std::time::Duration,
    ) -> Result<Vec<Node<'b>>, String> {
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            self.kittest.run();

            if !self.query_all_by_label(label).is_empty() {
                break;
            }
            // let mut labels = self.kittest.kittest_state().query_all_by_label(label);
            // if let Some(next) = labels.next() {
            //     tracing::trace!(?next, "Found label: {}", label);
            //     return Ok(next);
            // }
            // drop(labels);

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // query again to avoid borrow checker issues
        let nodes = self.query_all_by_label(label);
        if !nodes.is_empty() {
            tracing::trace!(?nodes, "Found label: {}", label);
            Ok(nodes)
        } else {
            Err(format!(
                "Label '{}' not found after waiting for {:?}",
                label, timeout
            ))
        }
    }
}
