use dash_sdk::dpp::dashcore::Network;
use std::time::Duration;

use crate::det_harness::DETHarness;

/// When I go to the Identities screen and click on the "Create Identity" button,
/// When I fill in the form with valid data,
/// Then I should see a success message indicating that the identity was created successfully.
#[test]
fn test_create_identity() {
    DETHarness::new("create_identity")
        .try_execute("create_identity", |det| {
            det.connect_to_network(Network::Testnet);

            det.click_by_label("I");
            det.click_by_label("Create Identity");
            // Ensure the instruction text is present
            det.get_by_label("Follow these steps to create your identity!");

            det.click_by_value("Select funding method");

            // det.set_text_by_label("Identity Name", "Test Identity");
            // TODO: this does not work, we cannot click here
            det.wait_all_by_label("Use Wallet Balance", Duration::from_secs(5))
                .expect("Wallet balance required");
            det.snapshot("identity_created");
            let success_nodes = det.query_all_by_label("Identity created successfully");
            tracing::debug!(
                count = success_nodes.len(),
                ?success_nodes,
                "Created identity"
            );
            assert!(!success_nodes.is_empty(), "Success message not found");
        })
        .expect("Failed to run create identity test");
}
