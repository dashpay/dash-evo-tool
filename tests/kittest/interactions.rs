use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::welcome_screen::WelcomeScreen;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Create a test harness with the standard configuration.
/// Returns the runtime (must be kept alive) and the harness.
/// Call `rt.enter()` on the returned runtime if the test triggers actions
/// that spawn tokio tasks (e.g., button clicks that dispatch AppActions).
fn create_test_harness() -> (
    tokio::runtime::Runtime,
    Harness<'static, dash_evo_tool::app::AppState>,
) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();
    let harness = Harness::builder().with_max_steps(200).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });
    (rt, harness)
}

/// Helper to dismiss the welcome screen and set up for main app testing.
fn dismiss_welcome_screen(harness: &mut Harness<'_, dash_evo_tool::app::AppState>) {
    harness.state_mut().show_welcome_screen = false;
    harness.state_mut().welcome_screen = None;
}

/// Helper to force-enable the welcome screen regardless of DB state.
fn enable_welcome_screen(harness: &mut Harness<'_, dash_evo_tool::app::AppState>) {
    let ctx = harness.state().mainnet_app_context.clone();
    harness.state_mut().show_welcome_screen = true;
    harness.state_mut().welcome_screen = Some(WelcomeScreen::new(ctx));
}

// =============================================================================
// Welcome Screen Rendering Tests
// =============================================================================

/// Test that when the welcome screen is enabled, it renders the welcome content
#[test]
fn test_welcome_screen_renders_when_enabled() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    enable_welcome_screen(&mut harness);
    harness.run_steps(10);

    assert!(
        harness.state().show_welcome_screen,
        "Welcome screen should be active"
    );

    let title = harness.query_by_label_contains("Welcome to Dash Evo Tool");
    assert!(
        title.is_some(),
        "Welcome screen title 'Welcome to Dash Evo Tool' should be visible"
    );
}

/// Test that the welcome screen shows its subtitle and instruction text
#[test]
fn test_welcome_screen_text_content() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    enable_welcome_screen(&mut harness);
    harness.run_steps(10);

    let subtitle = harness.query_by_label_contains("Your gateway to decentralized data");
    assert!(
        subtitle.is_some(),
        "Welcome screen subtitle should be visible"
    );

    let instruction = harness.query_by_label_contains("Select an option to get started");
    assert!(
        instruction.is_some(),
        "Welcome screen instruction text should be visible"
    );
}

/// Test that the welcome screen shows all three action cards
#[test]
fn test_welcome_screen_action_cards_present() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    enable_welcome_screen(&mut harness);
    harness.run_steps(10);

    // Action card titles
    assert!(
        harness.query_by_label_contains("Create Wallet").is_some(),
        "Create Wallet card should be visible"
    );
    assert!(
        harness.query_by_label_contains("Import Wallet").is_some(),
        "Import Wallet card should be visible"
    );
    assert!(
        harness.query_by_label_contains("Just Explore").is_some(),
        "Just Explore card should be visible"
    );

    // Card descriptions
    assert!(
        harness
            .query_by_label_contains("Start fresh with a new HD wallet")
            .is_some(),
        "Create Wallet description should be visible"
    );
    assert!(
        harness
            .query_by_label_contains("Load a wallet you already have")
            .is_some(),
        "Import Wallet description should be visible"
    );
    assert!(
        harness
            .query_by_label_contains("Explore without setting up")
            .is_some(),
        "Just Explore description should be visible"
    );
}

// =============================================================================
// Welcome Screen Click Tests
// (These need an active tokio runtime for the OnboardingComplete action)
// =============================================================================

/// Test that clicking "Just Explore" on the welcome screen dismisses it
#[test]
fn test_welcome_screen_just_explore_click() {
    let (rt, mut harness) = create_test_harness();
    let _guard = rt.enter();
    harness.set_size(egui::vec2(1024.0, 768.0));
    enable_welcome_screen(&mut harness);
    harness.run_steps(10);

    let explore_label = harness
        .query_by_label_contains("Explore without setting up")
        .expect("'Explore without setting up' label must be visible on welcome screen");
    explore_label.click();
    harness.run_steps(5);

    assert!(
        !harness.state().show_welcome_screen,
        "Welcome screen should be dismissed after clicking Just Explore"
    );
    assert_eq!(
        harness.state().selected_main_screen,
        RootScreenType::RootScreenDashPayProfile,
        "Should navigate to DashPay profile after Just Explore"
    );
}

/// Test that clicking "Create Wallet" navigates to wallets with add screen
#[test]
fn test_welcome_screen_create_wallet_click() {
    let (rt, mut harness) = create_test_harness();
    let _guard = rt.enter();
    harness.set_size(egui::vec2(1024.0, 768.0));
    enable_welcome_screen(&mut harness);
    harness.run_steps(10);

    let label = harness
        .query_by_label_contains("Start fresh with a new HD wallet")
        .expect("'Start fresh with a new HD wallet' label must be visible on welcome screen");
    label.click();
    harness.run_steps(5);

    assert!(
        !harness.state().show_welcome_screen,
        "Welcome screen should be dismissed after clicking Create Wallet"
    );
    assert_eq!(
        harness.state().selected_main_screen,
        RootScreenType::RootScreenWalletsBalances,
        "Should navigate to Wallets screen after Create Wallet"
    );
    assert!(
        !harness.state().screen_stack.is_empty(),
        "Screen stack should have the AddNewWallet screen pushed"
    );
}

/// Test that clicking "Import Wallet" navigates to wallets with import screen
#[test]
fn test_welcome_screen_import_wallet_click() {
    let (rt, mut harness) = create_test_harness();
    let _guard = rt.enter();
    harness.set_size(egui::vec2(1024.0, 768.0));
    enable_welcome_screen(&mut harness);
    harness.run_steps(10);

    let label = harness
        .query_by_label_contains("Load a wallet you already have")
        .expect("'Load a wallet you already have' label must be visible on welcome screen");
    label.click();
    harness.run_steps(5);

    assert!(
        !harness.state().show_welcome_screen,
        "Welcome screen should be dismissed after clicking Import Wallet"
    );
    assert_eq!(
        harness.state().selected_main_screen,
        RootScreenType::RootScreenWalletsBalances,
        "Should navigate to Wallets screen after Import Wallet"
    );
    assert!(
        !harness.state().screen_stack.is_empty(),
        "Screen stack should have the ImportMnemonic screen pushed"
    );
}

// =============================================================================
// Screen Navigation Tests
// =============================================================================

/// Test that programmatic screen switching renders each major screen without crashing
#[test]
fn test_switch_to_all_major_screens() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);
    harness.run_steps(5);

    let screens = [
        RootScreenType::RootScreenWalletsBalances,
        RootScreenType::RootScreenIdentities,
        RootScreenType::RootScreenDocumentQuery,
        RootScreenType::RootScreenMyTokenBalances,
        RootScreenType::RootScreenNetworkChooser,
        RootScreenType::RootScreenToolsPlatformInfoScreen,
        RootScreenType::RootScreenDashPayProfile,
        RootScreenType::RootScreenDPNSActiveContests,
        RootScreenType::RootScreenToolsProofLogScreen,
        RootScreenType::RootScreenToolsMasternodeListDiffScreen,
    ];

    for screen_type in screens {
        harness.state_mut().selected_main_screen = screen_type;
        harness.run_steps(10);

        assert_eq!(
            harness.state().selected_main_screen,
            screen_type,
            "Screen should be set to {:?}",
            screen_type
        );
    }
}

/// Test switching between screens preserves stability (no crash)
#[test]
fn test_screen_switching_round_trip() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
    harness.run_steps(10);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenIdentities;
    harness.run_steps(10);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenNetworkChooser;
    harness.run_steps(10);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
    harness.run_steps(10);

    assert_eq!(
        harness.state().selected_main_screen,
        RootScreenType::RootScreenWalletsBalances
    );
}

/// Test that rapid screen switching doesn't cause issues
#[test]
fn test_rapid_screen_switching() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);
    harness.run_steps(5);

    let screens = [
        RootScreenType::RootScreenWalletsBalances,
        RootScreenType::RootScreenIdentities,
        RootScreenType::RootScreenMyTokenBalances,
        RootScreenType::RootScreenDocumentQuery,
        RootScreenType::RootScreenNetworkChooser,
    ];

    for screen in screens.iter().cycle().take(20) {
        harness.state_mut().selected_main_screen = *screen;
        harness.run_steps(1);
    }

    harness.run_steps(5);
}

/// Test that the screen stack starts empty and remains empty on main screens
#[test]
fn test_screen_stack_empty_on_main_screens() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
    harness.run_steps(10);
    assert!(
        harness.state().screen_stack.is_empty(),
        "Screen stack should be empty on main wallets screen"
    );

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenIdentities;
    harness.run_steps(10);
    assert!(
        harness.state().screen_stack.is_empty(),
        "Screen stack should be empty on main identities screen"
    );
}

// =============================================================================
// Left Panel / UI Element Tests
// =============================================================================

/// Test that the left panel navigation labels are visible using contains-based matching
#[test]
fn test_left_panel_navigation_labels_visible() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);
    // Start on a screen that won't have conflicting labels
    harness.state_mut().selected_main_screen = RootScreenType::RootScreenDashPayProfile;
    harness.run_steps(15);

    // These labels appear in the left panel navigation.
    // We use query_all_by_label to allow multiple matches (e.g., label + screen content).
    let nav_labels = [
        "Wallets",
        "Identities",
        "Contracts",
        "Tokens",
        "Tools",
        "Settings",
    ];

    for label in nav_labels {
        let mut nodes = harness.query_all_by_label(label);
        assert!(
            nodes.next().is_some(),
            "Left panel should contain navigation label '{label}'"
        );
    }
}

/// Test that the wallets screen shows expected action buttons
#[test]
fn test_wallets_screen_has_action_buttons() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1280.0, 800.0));
    dismiss_welcome_screen(&mut harness);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenWalletsBalances;
    harness.run_steps(15);

    let mut import_nodes = harness.query_all_by_label_contains("Import Wallet");
    let mut create_nodes = harness.query_all_by_label_contains("Create Wallet");

    assert!(
        import_nodes.next().is_some() && create_nodes.next().is_some(),
        "Wallets screen should show both Import Wallet and Create Wallet buttons"
    );
}

/// Test that the network chooser screen shows expected configuration labels
#[test]
fn test_network_chooser_shows_config_labels() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    harness.state_mut().selected_main_screen = RootScreenType::RootScreenNetworkChooser;
    harness.run_steps(15);

    // The network chooser always shows "Network:" and only shows
    // "Connection Type:" when developer mode is enabled.
    let network_label = harness.query_by_label_contains("Network:");
    assert!(
        network_label.is_some(),
        "Network chooser should show 'Network:' label"
    );

    let connection_label = harness.query_by_label_contains("Connection Type:");
    let is_developer_mode = harness.state().current_app_context().is_developer_mode();
    if is_developer_mode {
        assert!(
            connection_label.is_some(),
            "Network chooser should show 'Connection Type:' label in developer mode"
        );
    } else {
        assert!(
            connection_label.is_none(),
            "Network chooser should hide 'Connection Type:' label when developer mode is disabled"
        );
    }
}

// =============================================================================
// Resize and Stress Tests
// =============================================================================

/// Test rendering at extreme window sizes doesn't crash any screen
#[test]
fn test_extreme_window_sizes_per_screen() {
    let (_rt, mut harness) = create_test_harness();
    dismiss_welcome_screen(&mut harness);

    let screens = [
        RootScreenType::RootScreenWalletsBalances,
        RootScreenType::RootScreenIdentities,
        RootScreenType::RootScreenNetworkChooser,
    ];

    let extreme_sizes = [
        egui::vec2(320.0, 240.0),   // Very small
        egui::vec2(3840.0, 2160.0), // 4K
    ];

    for screen in screens {
        for size in extreme_sizes {
            harness.state_mut().selected_main_screen = screen;
            harness.set_size(size);
            harness.run_steps(5);
        }
    }
}

/// Test DashPay-related screens render without crashing
#[test]
fn test_dashpay_screens_render() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    let dashpay_screens = [
        RootScreenType::RootScreenDashPayProfile,
        RootScreenType::RootScreenDashPayContacts,
        RootScreenType::RootScreenDashPayPayments,
    ];

    for screen in dashpay_screens {
        harness.state_mut().selected_main_screen = screen;
        harness.run_steps(10);
        assert_eq!(harness.state().selected_main_screen, screen);
    }
}

/// Test token-related screens render without crashing
#[test]
fn test_token_screens_render() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    let token_screens = [
        RootScreenType::RootScreenMyTokenBalances,
        RootScreenType::RootScreenTokenSearch,
        RootScreenType::RootScreenTokenCreator,
    ];

    for screen in token_screens {
        harness.state_mut().selected_main_screen = screen;
        harness.run_steps(10);
        assert_eq!(harness.state().selected_main_screen, screen);
    }
}

/// Test tools-related screens render without crashing
#[test]
fn test_tools_screens_render() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    let tools_screens = [
        RootScreenType::RootScreenToolsPlatformInfoScreen,
        RootScreenType::RootScreenToolsProofLogScreen,
        RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        RootScreenType::RootScreenToolsDocumentVisualizerScreen,
        RootScreenType::RootScreenToolsProofVisualizerScreen,
        RootScreenType::RootScreenToolsContractVisualizerScreen,
        RootScreenType::RootScreenToolsGroveSTARKScreen,
        RootScreenType::RootScreenToolsAddressBalanceScreen,
    ];

    for screen in tools_screens {
        harness.state_mut().selected_main_screen = screen;
        harness.run_steps(10);
        assert_eq!(harness.state().selected_main_screen, screen);
    }
}

/// Test DPNS-related screens render without crashing
#[test]
fn test_dpns_screens_render() {
    let (_rt, mut harness) = create_test_harness();
    harness.set_size(egui::vec2(1024.0, 768.0));
    dismiss_welcome_screen(&mut harness);

    let dpns_screens = [
        RootScreenType::RootScreenDPNSActiveContests,
        RootScreenType::RootScreenDPNSPastContests,
        RootScreenType::RootScreenDPNSOwnedNames,
        RootScreenType::RootScreenDPNSScheduledVotes,
    ];

    for screen in dpns_screens {
        harness.state_mut().selected_main_screen = screen;
        harness.run_steps(10);
        assert_eq!(harness.state().selected_main_screen, screen);
    }
}
