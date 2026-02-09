import { render, screen, fireEvent, waitFor } from "@testing-library/react";
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

// Build default mock commands
function createMockCommands() {
  return {
    getNetworkInfo: vi.fn().mockResolvedValue({
      activeNetwork: "testnet",
      availableNetworks: ["dash", "testnet", "devnet", "regtest"],
    }),
    settingsGet: vi.fn().mockResolvedValue({
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
    }),
    contextIsDeveloperMode: vi.fn().mockResolvedValue(false),
    settingsGetAutoStartSpv: vi.fn().mockResolvedValue({
      status: "ok",
      data: false,
    }),
    contextGetCoreBackendMode: vi.fn().mockResolvedValue("rpc"),
    getSpvStatus: vi.fn().mockResolvedValue([]),
    switchNetwork: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    contextSetCoreBackendMode: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    walletStartSpv: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    walletStopSpv: vi.fn().mockResolvedValue(undefined),
    walletClearSpvData: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    coreStartDashQt: vi.fn().mockResolvedValue(undefined),
    coreGetBestChainLocks: vi.fn().mockResolvedValue(undefined),
    contextEnableDeveloperMode: vi.fn().mockResolvedValue(undefined),
    settingsUpdateDashCore: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    settingsUpdateDisableZmq: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    settingsUpdateCloseDashQtOnExit: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    settingsUpdateAutoStartSpv: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    settingsUpdateOnboardingCompleted: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    systemWipePlatformData: vi.fn().mockResolvedValue(undefined),
    systemUpdateTheme: vi.fn().mockResolvedValue(undefined),
  };
}

let mockCommands = createMockCommands();

vi.mock("@/bindings", () => {
  const listenFn = vi.fn().mockResolvedValue(() => {});
  return {
    get commands() {
      return mockCommands;
    },
    events: {
      spvStatusEvent: { listen: listenFn },
      zmqConnectionStatusEvent: { listen: listenFn },
    },
  };
});

describe("NetworkChooserScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCommands = createMockCommands();
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
    mockCommands.contextIsDeveloperMode.mockResolvedValue(true);

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
    mockCommands.contextIsDeveloperMode.mockResolvedValue(true);

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
    mockCommands.contextIsDeveloperMode.mockResolvedValue(true);

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
      expect(mockCommands.settingsUpdateDashCore).toHaveBeenCalledWith({
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
      expect(mockCommands.settingsUpdateDisableZmq).toHaveBeenCalledWith(true);
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
      expect(mockCommands.contextEnableDeveloperMode).toHaveBeenCalledWith(
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
        mockCommands.settingsUpdateCloseDashQtOnExit,
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

  // Local network password

  it("shows local network password input when regtest + RPC", async () => {
    mockCommands.getNetworkInfo.mockResolvedValue({
      activeNetwork: "regtest",
      availableNetworks: ["dash", "regtest"],
    });
    mockCommands.settingsGet.mockResolvedValue({
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
    mockCommands.getNetworkInfo.mockResolvedValue({
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
      expect(mockCommands.getNetworkInfo).toHaveBeenCalled();
    });
  });

  it("calls settingsGet on mount", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(mockCommands.settingsGet).toHaveBeenCalled();
    });
  });

  it("calls contextIsDeveloperMode on mount", async () => {
    render(<NetworkChooserScreen />);
    await waitFor(() => {
      expect(mockCommands.contextIsDeveloperMode).toHaveBeenCalled();
    });
  });

  // Connect button interaction

  it("Connect button is hidden for regtest + RPC", async () => {
    mockCommands.getNetworkInfo.mockResolvedValue({
      activeNetwork: "regtest",
      availableNetworks: ["dash", "regtest"],
    });
    mockCommands.settingsGet.mockResolvedValue({
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
    mockCommands.contextIsDeveloperMode.mockResolvedValue(true);
    mockCommands.contextGetCoreBackendMode.mockResolvedValue("spv");
    mockCommands.settingsGet.mockResolvedValue({
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
    mockCommands.settingsGet.mockResolvedValue({
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
});
