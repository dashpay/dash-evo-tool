import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { NetworkChooserScreen } from "./NetworkChooserScreen";

// Mock @tanstack/react-router
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

// Mock theme provider
const mockSetTheme = vi.fn();
vi.mock("@/components/theme", () => ({
  useTheme: () => ({
    theme: "system" as const,
    resolvedTheme: "light" as const,
    setTheme: mockSetTheme,
  }),
}));

// Mock sonner
vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
  },
  Toaster: () => null,
}));

// Centralized bindings mock
vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

describe("NetworkChooserScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Set up default mock responses for NetworkChooserScreen
    vi.mocked(commands.getNetworkInfo).mockResolvedValue({
      activeNetwork: "testnet",
      availableNetworks: ["dash", "testnet", "devnet", "regtest"],
    });
    vi.mocked(commands.settingsGet).mockResolvedValue({
      status: "ok",
      data: {
        network: "testnet",
        themeMode: "system",
        overwriteDashConf: false,
        disableZmq: false,
        onboardingCompleted: true,
        showEvonodeTools: false,
        userMode: "beginner",
        closeDashQtOnExit: true,
        coreBackendMode: "rpc",
        hasPassword: false,
        dashQtPath: null,
      },
    });
    vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(false);
    vi.mocked(commands.settingsGetAutoStartSpv).mockResolvedValue({
      status: "ok",
      data: false,
    });
    vi.mocked(commands.contextGetCoreBackendMode).mockResolvedValue("rpc");
    vi.mocked(commands.getSpvStatus).mockResolvedValue([]);
  });

  it("renders the screen container", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("network-chooser-screen")).toBeInTheDocument();
    });
  });

  it("renders Connection Settings heading", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByText("Connection Settings"),
      ).toBeInTheDocument();
    });
  });

  it("renders Connection Status heading", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByText("Connection Status"),
      ).toBeInTheDocument();
    });
  });

  it("renders Advanced Settings toggle button", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByTestId("advanced-settings-toggle"),
      ).toBeInTheDocument();
    });
  });

  it("renders network badge", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("network-badge")).toBeInTheDocument();
    });
  });

  it("displays the current network from backend", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("network-badge")).toHaveTextContent("Testnet");
    });
  });

  it("shows network selector", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByTestId("network-select-trigger"),
      ).toBeInTheDocument();
    });
  });

  it("does NOT show connection type selector when not in developer mode", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByText("Connection Settings")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("connection-type-trigger")).not.toBeInTheDocument();
  });

  it("shows connection type selector when developer mode is enabled", async () => {
    vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(true);

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByTestId("connection-type-trigger"),
      ).toBeInTheDocument();
    });
  });

  it("shows Connect button when not connected in RPC mode", async () => {
    // With testnet + RPC, not connected, should show Connect
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("connect-button")).toBeInTheDocument();
    });
  });

  // Advanced Settings

  it("Advanced Settings section is collapsed by default", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByTestId("advanced-settings-toggle"),
      ).toBeInTheDocument();
    });
    // Check aria-expanded
    expect(screen.getByTestId("advanced-settings-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("clicking Advanced Settings toggle opens the section", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByTestId("advanced-settings-toggle"),
      ).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.getByTestId("advanced-settings-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("shows theme selector in Advanced Settings", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByTestId("advanced-settings-toggle"),
      ).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.getByTestId("theme-select-trigger")).toBeInTheDocument();
  });

  it("shows Dash Core path input in Advanced Settings", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.getByTestId("dashqt-path-input")).toBeInTheDocument();
  });

  it("shows configuration checkboxes in Advanced Settings", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.getByTestId("overwrite-dash-conf")).toBeInTheDocument();
    expect(screen.getByTestId("disable-zmq")).toBeInTheDocument();
    expect(screen.getByTestId("developer-mode")).toBeInTheDocument();
    expect(screen.getByTestId("close-dash-qt")).toBeInTheDocument();
  });

  it("shows database maintenance section in Advanced Settings", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.getByTestId("clear-database-button")).toBeInTheDocument();
  });

  it("does NOT show SPV maintenance when not in developer mode", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.queryByTestId("clear-spv-data-button")).not.toBeInTheDocument();
  });

  it("does NOT show developer tools section when developer mode is off", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.queryByTestId("clear-platform-addresses")).not.toBeInTheDocument();
  });

  it("shows developer tools when developer mode is enabled", async () => {
    vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(true);

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    await waitFor(() => {
      expect(
        screen.getByTestId("clear-platform-addresses"),
      ).toBeInTheDocument();
    });
  });

  it("shows auto-start SPV option when developer mode is enabled", async () => {
    vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(true);

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    await waitFor(() => {
      expect(screen.getByTestId("auto-start-spv")).toBeInTheDocument();
    });
  });

  // Checkbox interactions

  it("toggling overwrite dash.conf calls settingsUpdateDashCore", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    const checkbox = screen.getByTestId("overwrite-dash-conf").querySelector("input")!;
    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(commands.settingsUpdateDashCore).toHaveBeenCalledWith({
        customDashQtPath: null,
        overwriteDashConf: true,
      });
    });
  });

  it("toggling disable ZMQ calls settingsUpdateDisableZmq", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    const checkbox = screen.getByTestId("disable-zmq").querySelector("input")!;
    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(commands.settingsUpdateDisableZmq).toHaveBeenCalledWith(true);
    });
  });

  it("toggling developer mode calls contextEnableDeveloperMode", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    const checkbox = screen.getByTestId("developer-mode").querySelector("input")!;
    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(commands.contextEnableDeveloperMode).toHaveBeenCalledWith(
        true,
      );
    });
  });

  it("toggling close Dash-Qt calls settingsUpdateCloseDashQtOnExit", async () => {
    // Initial state has closeDashQtOnExit: true, unchecking it sends false
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    const checkbox = screen.getByTestId("close-dash-qt").querySelector("input")!;
    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(
        commands.settingsUpdateCloseDashQtOnExit,
      ).toHaveBeenCalledWith(false);
    });
  });

  // Database clear flow

  it("clicking Clear Database button opens confirmation dialog", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("clear-database-button"));

    await waitFor(() => {
      expect(screen.getByText("Clear Database")).toBeInTheDocument();
      expect(
        screen.getByText(/permanently deletes all local database entries/i),
      ).toBeInTheDocument();
    });
  });

  it("shows success feedback only after taskResultEvent fires for database wipe", async () => {
    vi.mocked(commands.systemWipePlatformData).mockResolvedValue({
      taskId: "wipe-1",
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    // Open advanced settings and click clear database
    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("clear-database-button"));

    // Confirm the dialog
    await waitFor(() => {
      expect(screen.getByText("Delete Data")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText("Delete Data"));

    // Wait for dispatch
    await waitFor(() => {
      expect(commands.systemWipePlatformData).toHaveBeenCalled();
    });

    // No success feedback yet — task is still running
    expect(screen.queryByTestId("feedback-success")).not.toBeInTheDocument();

    // Simulate backend completing the task
    await act(async () => {
      const listeners = vi.mocked(events.taskResultEvent.listen).mock.calls;
      const lastCb = listeners[listeners.length - 1][0];
      lastCb({ payload: { taskId: "wipe-1", result: { type: "systemCompleted" } } });
    });

    // Now the success feedback should appear
    await waitFor(() => {
      expect(screen.getByTestId("feedback-success")).toBeInTheDocument();
      expect(
        screen.getByText(/Cleared Testnet database/),
      ).toBeInTheDocument();
    });
  });

  it("shows error feedback when database wipe task fails", async () => {
    vi.mocked(commands.systemWipePlatformData).mockResolvedValue({
      taskId: "wipe-2",
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("clear-database-button"));

    await waitFor(() => {
      expect(screen.getByText("Delete Data")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText("Delete Data"));

    await waitFor(() => {
      expect(commands.systemWipePlatformData).toHaveBeenCalled();
    });

    // Simulate backend error
    await act(async () => {
      const listeners = vi.mocked(events.taskErrorEvent.listen).mock.calls;
      const lastCb = listeners[listeners.length - 1][0];
      lastCb({
        payload: {
          taskId: "wipe-2",
          domain: "system",
          message: "Database locked",
          details: "",
        },
      });
    });

    await waitFor(() => {
      expect(screen.getByTestId("feedback-error")).toBeInTheDocument();
      expect(
        screen.getByText(/Failed to clear database: Database locked/),
      ).toBeInTheDocument();
    });
  });

  it("shows error feedback when IPC dispatch itself fails", async () => {
    vi.mocked(commands.systemWipePlatformData).mockRejectedValue(
      new Error("IPC unavailable"),
    );

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("clear-database-button"));

    await waitFor(() => {
      expect(screen.getByText("Delete Data")).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText("Delete Data"));

    await waitFor(() => {
      expect(screen.getByTestId("feedback-error")).toBeInTheDocument();
      expect(
        screen.getByText(/Failed to clear database/),
      ).toBeInTheDocument();
    });
  });

  // Local network password

  it("shows local network password input when regtest + RPC", async () => {
    vi.mocked(commands.getNetworkInfo).mockResolvedValue({
      activeNetwork: "regtest",
      availableNetworks: ["dash", "regtest"],
    });
    vi.mocked(commands.settingsGet).mockResolvedValue({
      status: "ok",
      data: {
        network: "regtest",
        themeMode: "system",
        overwriteDashConf: false,
        disableZmq: false,
        onboardingCompleted: true,
        showEvonodeTools: false,
        userMode: "beginner",
        closeDashQtOnExit: true,
        coreBackendMode: "rpc",
        hasPassword: false,
        dashQtPath: null,
      },
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("local-password-input")).toBeInTheDocument();
    });
  });

  it("does NOT show local password when on mainnet", async () => {
    vi.mocked(commands.getNetworkInfo).mockResolvedValue({
      activeNetwork: "dash",
      availableNetworks: ["dash"],
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByText("Connection Settings")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("local-password-input")).not.toBeInTheDocument();
  });

  // Loads settings from backend on mount

  it("calls getNetworkInfo on mount", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(commands.getNetworkInfo).toHaveBeenCalled();
    });
  });

  it("calls settingsGet on mount", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(commands.settingsGet).toHaveBeenCalled();
    });
  });

  it("calls contextIsDeveloperMode on mount", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(commands.contextIsDeveloperMode).toHaveBeenCalled();
    });
  });

  // Connect button interaction

  it("Connect button is hidden for regtest + RPC", async () => {
    vi.mocked(commands.getNetworkInfo).mockResolvedValue({
      activeNetwork: "regtest",
      availableNetworks: ["dash", "regtest"],
    });
    vi.mocked(commands.settingsGet).mockResolvedValue({
      status: "ok",
      data: {
        network: "regtest",
        themeMode: "system",
        overwriteDashConf: false,
        disableZmq: false,
        onboardingCompleted: true,
        showEvonodeTools: false,
        userMode: "beginner",
        closeDashQtOnExit: true,
        coreBackendMode: "rpc",
        hasPassword: false,
        dashQtPath: null,
      },
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByText("Connection Status")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("connect-button")).not.toBeInTheDocument();
  });

  // SPV-specific behavior

  it("shows SPV experimental warning when SPV mode selected in dev mode", async () => {
    vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(true);
    vi.mocked(commands.contextGetCoreBackendMode).mockResolvedValue("spv");
    vi.mocked(commands.settingsGet).mockResolvedValue({
      status: "ok",
      data: {
        network: "testnet",
        themeMode: "system",
        overwriteDashConf: false,
        disableZmq: false,
        onboardingCompleted: true,
        showEvonodeTools: false,
        userMode: "beginner",
        closeDashQtOnExit: true,
        coreBackendMode: "spv",
        hasPassword: false,
        dashQtPath: null,
      },
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(
        screen.getByText("SPV mode is experimental and still in development"),
      ).toBeInTheDocument();
    });
  });

  it("displays Dash-Qt path when configured", async () => {
    vi.mocked(commands.settingsGet).mockResolvedValue({
      status: "ok",
      data: {
        network: "testnet",
        themeMode: "system",
        overwriteDashConf: false,
        disableZmq: false,
        onboardingCompleted: true,
        showEvonodeTools: false,
        userMode: "beginner",
        closeDashQtOnExit: true,
        coreBackendMode: "rpc",
        hasPassword: false,
        dashQtPath: "/usr/local/bin/dash-qt",
      },
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    await waitFor(() => {
      expect(screen.getByText("Path: /usr/local/bin/dash-qt")).toBeInTheDocument();
    });
  });

  // Browse button and file picker

  it("shows Browse button in Advanced Settings", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));

    expect(screen.getByTestId("dashqt-browse-button")).toBeInTheDocument();
  });

  it("Browse button calls settingsPickDashQtPath and auto-saves on valid path", async () => {
    vi.mocked(commands.settingsPickDashQtPath).mockResolvedValue({
      path: "/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt", error: null,
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("dashqt-browse-button"));

    await waitFor(() => {
      expect(commands.settingsPickDashQtPath).toHaveBeenCalled();
    });

    await waitFor(() => {
      expect(commands.settingsUpdateDashCore).toHaveBeenCalledWith({
        customDashQtPath: "/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt",
        overwriteDashConf: false,
      });
    });
  });

  it("Browse button shows error when invalid file is selected", async () => {
    vi.mocked(commands.settingsPickDashQtPath).mockResolvedValue({
      path: null, error: "Invalid file: Please select a valid 'Dash-Qt or Dash-Qt.app'.",
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("dashqt-browse-button"));

    await waitFor(() => {
      expect(screen.getByTestId("dashqt-path-error")).toBeInTheDocument();
      expect(
        screen.getByText("Invalid file: Please select a valid 'Dash-Qt or Dash-Qt.app'."),
      ).toBeInTheDocument();
    });
  });

  it("error can be dismissed", async () => {
    vi.mocked(commands.settingsPickDashQtPath).mockResolvedValue({
      path: null, error: "Invalid file",
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("dashqt-browse-button"));

    await waitFor(() => {
      expect(screen.getByTestId("dashqt-path-error")).toBeInTheDocument();
    });

    // Click the dismiss button (X icon inside the error)
    const dismissBtn = screen.getByTestId("dashqt-path-error").querySelector("button")!;
    fireEvent.click(dismissBtn);

    await waitFor(() => {
      expect(screen.queryByTestId("dashqt-path-error")).not.toBeInTheDocument();
    });
  });

  it("Browse does nothing when user cancels file dialog", async () => {
    vi.mocked(commands.settingsPickDashQtPath).mockResolvedValue({
      path: null, error: null,
    });

    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(screen.getByTestId("advanced-settings-toggle")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("advanced-settings-toggle"));
    fireEvent.click(screen.getByTestId("dashqt-browse-button"));

    await waitFor(() => {
      expect(commands.settingsPickDashQtPath).toHaveBeenCalled();
    });

    // Should not call settingsUpdateDashCore or show error
    expect(commands.settingsUpdateDashCore).not.toHaveBeenCalled();
    expect(screen.queryByTestId("dashqt-path-error")).not.toBeInTheDocument();
  });
});
