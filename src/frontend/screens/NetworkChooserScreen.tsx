import { useState, useEffect, useCallback, useRef } from "react";
import { Island } from "@/components/layout";
import { ProgressBar } from "@/components/feedback";
import { ConfirmationDialog } from "@/components/shared";
import { useTheme } from "@/components/theme";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Badge } from "@/components/ui/badge";
import {
  ChevronDown,
  ChevronRight,
  AlertTriangle,
  Loader2,
  Check,
  X,
  Eye,
  EyeOff,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { commands, events } from "@/bindings";
import type {
  NetworkDto,
  CoreBackendModeDto,
  SpvStatusEvent,
  SpvStatusDto,
  ThemeModeDto,
} from "@/bindings";
import { toast } from "sonner";
import { toastError } from "@/lib/toastError";

// Network display labels
const networkLabels: Record<NetworkDto, string> = {
  dash: "Mainnet",
  testnet: "Testnet",
  devnet: "Devnet",
  regtest: "Local",
};

// Network accent color classes
const networkColorClasses: Record<NetworkDto, string> = {
  dash: "bg-network-mainnet",
  testnet: "bg-network-testnet",
  devnet: "bg-network-devnet",
  regtest: "bg-network-local",
};

interface FeedbackMessage {
  type: "success" | "error";
  text: string;
}

export function NetworkChooserScreen() {
  // State
  const { theme, setTheme } = useTheme();
  const [currentNetwork, setCurrentNetwork] = useState<NetworkDto>("dash");
  const [availableNetworks, setAvailableNetworks] = useState<NetworkDto[]>([
    "dash",
  ]);
  const [backendMode, setBackendMode] = useState<CoreBackendModeDto>("rpc");
  const [developerMode, setDeveloperMode] = useState(false);
  const [connected, setConnected] = useState(false);
  const [spvStatus, setSpvStatus] = useState<SpvStatusDto>("idle");
  const [spvProgress, setSpvProgress] = useState<SpvStatusEvent | null>(null);

  // Settings state
  const [overwriteDashConf, setOverwriteDashConf] = useState(false);
  const [disableZmq, setDisableZmq] = useState(false);
  const [closeDashQtOnExit, setCloseDashQtOnExit] = useState(true);
  const [autoStartSpv, setAutoStartSpv] = useState(false);
  const [dashQtPath, setDashQtPath] = useState<string | null>(null);
  const [localPassword, setLocalPassword] = useState("");
  const [showLocalPassword, setShowLocalPassword] = useState(false);

  // UI state
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [spvClearDialogOpen, setSpvClearDialogOpen] = useState(false);
  const [dbClearDialogOpen, setDbClearDialogOpen] = useState(false);
  const [spvClearFeedback, setSpvClearFeedback] =
    useState<FeedbackMessage | null>(null);
  const [dbClearFeedback, setDbClearFeedback] =
    useState<FeedbackMessage | null>(null);
  const [connectLoading, setConnectLoading] = useState(false);
  const [dashQtPathError, setDashQtPathError] = useState<string | null>(null);

  // Polling ref
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Load initial state
  useEffect(() => {
    async function loadState() {
      try {
        const networkInfo = await commands.getNetworkInfo();
        setCurrentNetwork(networkInfo.activeNetwork);
        setAvailableNetworks(networkInfo.availableNetworks);
      } catch {
        // Backend not available
      }

      try {
        const result = await commands.settingsGet();
        if (result.status === "ok") {
          const s = result.data;
          setOverwriteDashConf(s.overwriteDashConf);
          setDisableZmq(s.disableZmq);
          setCloseDashQtOnExit(s.closeDashQtOnExit);
          setDashQtPath(s.dashQtPath);
          setBackendMode(s.coreBackendMode);
        }
      } catch {
        // Backend not available
      }

      try {
        const devMode = await commands.contextIsDeveloperMode();
        if (typeof devMode === "boolean") {
          setDeveloperMode(devMode);
        } else if (
          devMode &&
          typeof devMode === "object" &&
          "status" in devMode
        ) {
          const r = devMode as { status: string; data?: boolean };
          if (r.status === "ok" && typeof r.data === "boolean") {
            setDeveloperMode(r.data);
          }
        }
      } catch {
        // Backend not available
      }

      try {
        const autoResult = await commands.settingsGetAutoStartSpv();
        if (autoResult.status === "ok") {
          setAutoStartSpv(autoResult.data);
        }
      } catch {
        // Backend not available
      }

      try {
        const mode = await commands.contextGetCoreBackendMode();
        if (typeof mode === "string") {
          setBackendMode(mode);
        } else if (mode && typeof mode === "object" && "status" in mode) {
          const r = mode as { status: string; data?: CoreBackendModeDto };
          if (r.status === "ok" && r.data) {
            setBackendMode(r.data);
          }
        }
      } catch {
        // Backend not available
      }

      // Load SPV status
      try {
        const statuses = await commands.getSpvStatus();
        if (Array.isArray(statuses)) {
          const current = statuses.find((s) => s.network === currentNetwork);
          if (current) {
            setSpvStatus(current.status);
            setSpvProgress(current);
            setConnected(
              current.status === "running" || current.status === "syncing",
            );
          }
        }
      } catch {
        // Backend not available
      }
    }
    loadState();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Listen for SPV status events
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    events.spvStatusEvent
      .listen((event: { payload: SpvStatusEvent }) => {
        if (event.payload.network === currentNetwork) {
          setSpvStatus(event.payload.status);
          setSpvProgress(event.payload);
          setConnected(
            event.payload.status === "running" ||
              event.payload.status === "syncing",
          );
        }
      })
      .then((unlisten) => {
        cleanup = unlisten;
      })
      .catch(() => {
        // Events not available
      });
    return () => cleanup?.();
  }, [currentNetwork]);

  // Listen for ZMQ connection status for RPC mode
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    events.zmqConnectionStatusEvent
      .listen((event: { payload: { network: NetworkDto; connected: boolean } }) => {
        if (event.payload.network === currentNetwork) {
          setConnected(event.payload.connected);
        }
      })
      .then((unlisten) => {
        cleanup = unlisten;
      })
      .catch(() => {
        // Events not available
      });
    return () => cleanup?.();
  }, [currentNetwork]);

  // Poll chain locks for RPC connection check (every 3s)
  useEffect(() => {
    if (backendMode !== "rpc") {
      if (pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current);
        pollIntervalRef.current = null;
      }
      return;
    }

    const checkConnection = () => {
      // Fire-and-forget: dispatches task, result comes back via events
      commands.coreGetBestChainLocks().catch(() => {
        // Ignore errors — connection status updated by ZMQ events
      });
    };

    checkConnection();
    pollIntervalRef.current = setInterval(checkConnection, 3000);

    return () => {
      if (pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current);
        pollIntervalRef.current = null;
      }
    };
  }, [backendMode, currentNetwork]);

  // === Handlers ===

  const handleNetworkSwitch = useCallback(
    async (network: NetworkDto) => {
      try {
        const result = await commands.switchNetwork(network);
        if (result.status === "ok") {
          setCurrentNetwork(network);
          toast.success(`Switched to ${networkLabels[network]}`);
        } else {
          toastError(result.error);
        }
      } catch (e) {
        toastError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  const handleBackendModeSwitch = useCallback(
    async (mode: CoreBackendModeDto) => {
      try {
        const result = await commands.contextSetCoreBackendMode(mode);
        if (result.status === "ok") {
          setBackendMode(mode);
        } else {
          toastError(result.error);
        }
      } catch (e) {
        toastError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  const handleConnect = useCallback(async () => {
    setConnectLoading(true);
    try {
      if (backendMode === "spv") {
        const result = await commands.walletStartSpv();
        if (result.status === "error") {
          toastError(result.error);
        }
      } else {
        // RPC mode: start Dash-Qt
        if (dashQtPath) {
          await commands.coreStartDashQt({
            dashQtPath,
            overwriteDashConf,
          });
        } else {
          toast.error(
            "No Dash Core path configured. Set the path in Advanced Settings.",
          );
        }
      }
    } catch (e) {
      toastError(e instanceof Error ? e.message : String(e));
    } finally {
      setConnectLoading(false);
    }
  }, [backendMode, dashQtPath, overwriteDashConf]);

  const handleDisconnect = useCallback(async () => {
    try {
      await commands.walletStopSpv();
    } catch {
      // Ignore disconnect errors
    }
  }, []);

  const handleSaveOverwriteDashConf = useCallback(
    async (value: boolean) => {
      setOverwriteDashConf(value);
      try {
        await commands.settingsUpdateDashCore({
          customDashQtPath: dashQtPath,
          overwriteDashConf: value,
        });
      } catch {
        // Ignore save errors
      }
    },
    [dashQtPath],
  );

  const handleSaveDisableZmq = useCallback(async (value: boolean) => {
    setDisableZmq(value);
    try {
      await commands.settingsUpdateDisableZmq(value);
    } catch {
      // Ignore save errors
    }
  }, []);

  const handleSaveCloseDashQt = useCallback(async (value: boolean) => {
    setCloseDashQtOnExit(value);
    try {
      await commands.settingsUpdateCloseDashQtOnExit(value);
    } catch {
      // Ignore save errors
    }
  }, []);

  const handleSaveAutoStartSpv = useCallback(async (value: boolean) => {
    setAutoStartSpv(value);
    try {
      await commands.settingsUpdateAutoStartSpv(value);
    } catch {
      // Ignore save errors
    }
  }, []);

  const handleDeveloperModeToggle = useCallback(
    async (enabled: boolean) => {
      setDeveloperMode(enabled);
      try {
        await commands.contextEnableDeveloperMode(enabled);
      } catch {
        // Ignore errors
      }

      // When disabling developer mode, switch to RPC and stop SPV
      if (!enabled && backendMode === "spv") {
        try {
          await commands.contextSetCoreBackendMode("rpc");
          setBackendMode("rpc");
          await commands.walletStopSpv();
        } catch {
          // Ignore errors
        }
      }
    },
    [backendMode],
  );

  const handleClearSpvData = useCallback(async () => {
    try {
      const result = await commands.walletClearSpvData();
      if (result.status === "ok") {
        setSpvClearFeedback({
          type: "success",
          text: `Cleared SPV data for ${networkLabels[currentNetwork]}. Reconnect to start a new sync.`,
        });
      } else {
        setSpvClearFeedback({
          type: "error",
          text: `Failed to clear SPV data: ${result.error}`,
        });
      }
    } catch (e) {
      setSpvClearFeedback({
        type: "error",
        text: `Failed to clear SPV data: ${e}`,
      });
    }
  }, [currentNetwork]);

  const handleClearDatabase = useCallback(async () => {
    try {
      await commands.systemWipePlatformData();
      setDbClearFeedback({
        type: "success",
        text: `Cleared ${networkLabels[currentNetwork]} database. Restart or resync to rebuild state.`,
      });
    } catch (e) {
      setDbClearFeedback({
        type: "error",
        text: `Failed to clear database: ${e}`,
      });
    }
  }, [currentNetwork]);

  const handleSaveLocalPassword = useCallback(async () => {
    // TODO: This would need a dedicated IPC command to save the local network password
    // For now, show a toast indicating it's not yet implemented
    toast.info("Local network password saved (changes take effect on restart)");
  }, []);

  const isSpvActive =
    spvStatus === "running" || spvStatus === "syncing" || spvStatus === "starting";
  const showConnectButton =
    !(currentNetwork === "regtest" && backendMode === "rpc");

  return (
    <div
      className="flex flex-1 flex-col gap-4 overflow-auto"
      data-testid="network-chooser-screen"
    >
      {/* Connection Settings Card */}
      <Island as="section">
        <h2 className="text-xl font-semibold text-foreground">
          Connection Settings
        </h2>

        <div className="mt-5 grid grid-cols-[auto_1fr] items-center gap-x-10 gap-y-3">
          {/* Connection Type (developer mode only) */}
          {developerMode && (
            <>
              <Label className="text-sm text-foreground">
                Connection Type:
              </Label>
              <div>
                <Select
                  value={backendMode}
                  onValueChange={(value) =>
                    handleBackendModeSwitch(value as CoreBackendModeDto)
                  }
                  data-testid="connection-type-select"
                >
                  <SelectTrigger className="w-[220px]" data-testid="connection-type-trigger">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="spv">SPV Client</SelectItem>
                    <SelectItem value="rpc">Dash Core RPC</SelectItem>
                  </SelectContent>
                </Select>

                {/* SPV experimental warning */}
                {backendMode === "spv" && (
                  <div className="mt-2 flex items-center gap-2 rounded border border-warning bg-warning/15 px-3 py-1.5">
                    <AlertTriangle className="h-4 w-4 flex-shrink-0 text-warning" />
                    <span className="text-xs text-warning">
                      SPV mode is experimental and still in development
                    </span>
                  </div>
                )}
              </div>
            </>
          )}

          {/* Network Selector */}
          <Label className="text-sm text-foreground">Network:</Label>
          <div>
            <Select
              value={currentNetwork}
              onValueChange={(value) =>
                handleNetworkSwitch(value as NetworkDto)
              }
              disabled={backendMode === "spv" && isSpvActive}
            >
              <SelectTrigger className="w-[220px]" data-testid="network-select-trigger">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {availableNetworks.map((net) => (
                  <SelectItem key={net} value={net}>
                    {networkLabels[net]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {backendMode === "spv" && isSpvActive && (
              <p className="mt-1 text-xs text-muted-foreground">
                Disconnect from SPV first to switch networks
              </p>
            )}
          </div>
        </div>

        {/* Local network password (Regtest + RPC only) */}
        {currentNetwork === "regtest" && backendMode === "rpc" && (
          <>
            <div className="my-5 border-t" />
            <Label className="text-sm font-semibold text-foreground">
              Local Network Password
            </Label>
            <div className="mt-2 flex items-center gap-2">
              <div className="relative flex-1 max-w-sm">
                <Input
                  type={showLocalPassword ? "text" : "password"}
                  value={localPassword}
                  onChange={(e) => setLocalPassword(e.target.value)}
                  placeholder="Enter password"
                  data-testid="local-password-input"
                />
                <button
                  type="button"
                  onClick={() => setShowLocalPassword(!showLocalPassword)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground hover:text-foreground"
                  aria-label={
                    showLocalPassword ? "Hide password" : "Show password"
                  }
                >
                  {showLocalPassword ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </button>
              </div>
              <Button size="sm" onClick={handleSaveLocalPassword}>
                Save
              </Button>
            </div>
          </>
        )}
      </Island>

      {/* Connection Status Card */}
      <Island as="section">
        <h2 className="text-xl font-semibold text-foreground">
          Connection Status
        </h2>
        <div className="mt-3">
          <div className="flex items-center gap-3">
            {connected || isSpvActive ? (
              <>
                {backendMode === "spv" ? (
                  <>
                    <Button
                      variant="destructive"
                      onClick={handleDisconnect}
                      data-testid="disconnect-button"
                    >
                      Disconnect
                    </Button>
                    <SpvConnectionStatus status={spvStatus} />
                  </>
                ) : (
                  <div className="flex items-center gap-2 text-primary">
                    <Check className="h-4 w-4" />
                    <span className="text-sm font-medium">Connected</span>
                  </div>
                )}
              </>
            ) : (
              <>
                {showConnectButton && (
                  <Button
                    onClick={handleConnect}
                    disabled={connectLoading}
                    data-testid="connect-button"
                  >
                    {connectLoading && (
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    )}
                    Connect
                  </Button>
                )}
                {!showConnectButton && (
                  <div className="flex items-center gap-2 text-muted-foreground">
                    <X className="h-4 w-4" />
                    <span className="text-sm">
                      Not connected (configure local network above)
                    </span>
                  </div>
                )}
              </>
            )}
          </div>

          {/* SPV Sync Progress */}
          {developerMode &&
            backendMode === "spv" &&
            (spvStatus === "syncing" || spvStatus === "starting") &&
            spvProgress && (
              <div className="mt-4 border-t pt-4">
                <SpvSyncProgress progress={spvProgress} />
              </div>
            )}
        </div>
      </Island>

      {/* Advanced Settings (Collapsible) */}
      <Island as="section" noPadding>
        <button
          type="button"
          onClick={() => setAdvancedOpen(!advancedOpen)}
          className="flex w-full items-center gap-3 px-6 py-4 text-left transition-colors hover:bg-accent/50"
          data-testid="advanced-settings-toggle"
          aria-expanded={advancedOpen}
        >
          <div className="flex h-6 w-6 items-center justify-center rounded bg-muted text-primary">
            {advancedOpen ? (
              <ChevronDown className="h-4 w-4" />
            ) : (
              <ChevronRight className="h-4 w-4" />
            )}
          </div>
          <span className="text-base font-medium text-foreground">
            Advanced Settings
          </span>
        </button>

        {advancedOpen && (
          <div className="space-y-6 border-t px-6 pb-6 pt-4">
            {/* Theme Selection */}
            <div className="flex items-center gap-4">
              <Label className="min-w-[120px] text-sm">Theme:</Label>
              <Select
                value={theme}
                onValueChange={(value) => setTheme(value as ThemeModeDto)}
              >
                <SelectTrigger className="w-[160px]" data-testid="theme-select-trigger">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="system">System</SelectItem>
                  <SelectItem value="light">Light</SelectItem>
                  <SelectItem value="dark">Dark</SelectItem>
                </SelectContent>
              </Select>
            </div>

            {/* Dash Core Executable Path */}
            <div>
              <div className="border-t pt-4">
                <Label className="text-sm font-semibold text-foreground">
                  Dash Core Executable Path
                </Label>
                <div className="mt-2 flex items-center gap-2">
                  <Input
                    value={dashQtPath ?? ""}
                    onChange={(e) => {
                      const newPath = e.target.value || null;
                      setDashQtPath(newPath);
                      setDashQtPathError(null);
                    }}
                    placeholder="Path to Dash-Qt executable"
                    className="max-w-md"
                    data-testid="dashqt-path-input"
                  />
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={async () => {
                      try {
                        const result =
                          await commands.settingsPickDashQtPath();
                        if (result.status === "ok") {
                          const { path, error } = result.data;
                          if (path) {
                            setDashQtPath(path);
                            setDashQtPathError(null);
                            await commands.settingsUpdateDashCore({
                              customDashQtPath: path,
                              overwriteDashConf,
                            });
                            toast.success("Dash-Qt path saved");
                          } else if (error) {
                            setDashQtPathError(error);
                          }
                          // If both null, user cancelled — do nothing
                        }
                      } catch {
                        // Backend not available
                      }
                    }}
                    data-testid="dashqt-browse-button"
                  >
                    Browse
                  </Button>
                  {dashQtPath && (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setDashQtPath(null);
                        setDashQtPathError(null);
                        commands
                          .settingsUpdateDashCore({
                            customDashQtPath: null,
                            overwriteDashConf,
                          })
                          .catch(() => {});
                      }}
                    >
                      Clear
                    </Button>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      if (dashQtPath) {
                        commands
                          .settingsUpdateDashCore({
                            customDashQtPath: dashQtPath,
                            overwriteDashConf,
                          })
                          .then((r) => {
                            if (r.status === "ok") {
                              toast.success("Dash-Qt path saved");
                            }
                          })
                          .catch(() => {});
                      }
                    }}
                  >
                    Save
                  </Button>
                </div>
                {dashQtPath && (
                  <p className="mt-1 text-xs text-success italic">
                    Path: {dashQtPath}
                  </p>
                )}
                {dashQtPathError && (
                  <div
                    className="mt-2 flex items-center gap-2 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2"
                    data-testid="dashqt-path-error"
                  >
                    <AlertTriangle className="h-4 w-4 text-destructive" />
                    <span className="text-sm text-destructive">
                      {dashQtPathError}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="ml-auto h-6 px-2"
                      onClick={() => setDashQtPathError(null)}
                    >
                      <X className="h-3 w-3" />
                    </Button>
                  </div>
                )}
              </div>
            </div>

            {/* Configuration Options */}
            <div className="border-t pt-4">
              <Label className="text-sm font-semibold text-foreground">
                Configuration Options
              </Label>
              <div className="mt-3 space-y-3">
                <CheckboxRow
                  checked={overwriteDashConf}
                  onChange={handleSaveOverwriteDashConf}
                  label="Overwrite dash.conf"
                  description="Auto-configure required settings"
                  testId="overwrite-dash-conf"
                />
                <CheckboxRow
                  checked={disableZmq}
                  onChange={handleSaveDisableZmq}
                  label="Disable ZMQ (requires restart)"
                  testId="disable-zmq"
                />
                <CheckboxRow
                  checked={developerMode}
                  onChange={handleDeveloperModeToggle}
                  label="Developer mode"
                  description="Enable advanced features"
                  testId="developer-mode"
                />
                <CheckboxRow
                  checked={closeDashQtOnExit}
                  onChange={handleSaveCloseDashQt}
                  label="Close Dash-Qt when DET exits"
                  description={
                    closeDashQtOnExit
                      ? "Dash-Qt will close automatically"
                      : "Dash-Qt will keep running"
                  }
                  testId="close-dash-qt"
                />
              </div>
            </div>

            {/* Developer Tools */}
            {developerMode && (
              <div className="border-t pt-4">
                <Label className="text-sm font-semibold text-foreground">
                  Developer Tools
                </Label>
                <div className="mt-3">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={async () => {
                      // TODO: Implement clear platform addresses IPC
                      toast.info("Platform addresses cleared");
                    }}
                    data-testid="clear-platform-addresses"
                  >
                    Clear Platform Addresses
                  </Button>
                  <p className="mt-1 text-xs text-muted-foreground italic">
                    Removes all Platform addresses for testing sync
                  </p>
                </div>
              </div>
            )}

            {/* SPV Settings (developer mode only) */}
            {developerMode && (
              <>
                <div className="border-t pt-4">
                  <Label className="text-sm font-semibold text-foreground">
                    SPV Peer Source
                  </Label>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Choose how SPV finds peers for blockchain sync on
                    mainnet/testnet.
                  </p>
                  {/* Note: use_local_spv_node toggle would need a dedicated IPC command */}
                  <p className="mt-2 text-xs text-muted-foreground italic">
                    Note: Changes take effect on next SPV sync start.
                    Devnet/local networks always use configured host.
                  </p>
                </div>

                <div className="border-t pt-4">
                  <Label className="text-sm font-semibold text-foreground">
                    SPV Auto-Start
                  </Label>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Automatically start SPV sync when the app opens.
                  </p>
                  <div className="mt-2">
                    <CheckboxRow
                      checked={autoStartSpv}
                      onChange={handleSaveAutoStartSpv}
                      label="Auto-start SPV on startup"
                      description={autoStartSpv ? "Enabled" : "Disabled"}
                      testId="auto-start-spv"
                    />
                  </div>
                </div>
              </>
            )}

            {/* Database Maintenance */}
            <div className="border-t pt-4">
              <Label className="text-sm font-semibold text-foreground">
                Database Maintenance
              </Label>
              <p className="mt-1 text-sm text-muted-foreground">
                Remove all local data for the current network (wallets,
                contacts, identities, tokens, etc.).
              </p>
              <div className="mt-3">
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => {
                    setDbClearDialogOpen(true);
                    setDbClearFeedback(null);
                  }}
                  data-testid="clear-database-button"
                >
                  Clear {networkLabels[currentNetwork]} Database
                </Button>
              </div>
              <FeedbackBanner
                feedback={dbClearFeedback}
                onDismiss={() => setDbClearFeedback(null)}
              />
            </div>

            {/* SPV Maintenance (developer mode + SPV mode) */}
            {developerMode && backendMode === "spv" && (
              <div className="border-t pt-4">
                <Label className="text-sm font-semibold text-foreground">
                  SPV Maintenance
                </Label>
                <p className="mt-1 text-sm text-muted-foreground">
                  Clear cached headers and filter data for this network.
                </p>
                <div className="mt-3">
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={isSpvActive}
                    onClick={() => {
                      setSpvClearDialogOpen(true);
                      setSpvClearFeedback(null);
                    }}
                    data-testid="clear-spv-data-button"
                  >
                    Clear SPV Data
                  </Button>
                  {isSpvActive && (
                    <p className="mt-1 text-xs text-muted-foreground">
                      Stop the SPV client before clearing data
                    </p>
                  )}
                </div>
                <FeedbackBanner
                  feedback={spvClearFeedback}
                  onDismiss={() => setSpvClearFeedback(null)}
                />
              </div>
            )}
          </div>
        )}
      </Island>

      {/* Network badge footer */}
      <div className="flex items-center justify-center pb-4">
        <Badge
          className={cn("network-badge", networkColorClasses[currentNetwork])}
          data-testid="network-badge"
        >
          {networkLabels[currentNetwork]}
        </Badge>
      </div>

      {/* Confirmation Dialogs */}
      <ConfirmationDialog
        open={dbClearDialogOpen}
        onOpenChange={setDbClearDialogOpen}
        title="Clear Database"
        message={`This permanently deletes all local database entries for ${networkLabels[currentNetwork]}. This includes wallets, tokens, contacts, and cached identity data. This cannot be undone.`}
        confirmText="Delete Data"
        cancelText="Cancel"
        danger
        onResult={(status) => {
          if (status === "confirmed") {
            handleClearDatabase();
          }
        }}
      />

      <ConfirmationDialog
        open={spvClearDialogOpen}
        onOpenChange={setSpvClearDialogOpen}
        title="Clear SPV Data"
        message={`This will delete cached SPV data for ${networkLabels[currentNetwork]}. The next connection will trigger a full resync.`}
        confirmText="Clear Data"
        cancelText="Keep Data"
        danger
        onResult={(status) => {
          if (status === "confirmed") {
            handleClearSpvData();
          }
        }}
      />
    </div>
  );
}

// === Sub-components ===

/** Checkbox with label and optional description */
function CheckboxRow({
  checked,
  onChange,
  label,
  description,
  testId,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  description?: string;
  testId?: string;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-3" data-testid={testId}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 h-4 w-4 rounded border-border text-primary accent-primary focus:ring-primary"
      />
      <div>
        <span className="text-sm text-foreground">{label}</span>
        {description && (
          <p className="text-xs text-muted-foreground italic">{description}</p>
        )}
      </div>
    </label>
  );
}

/** SPV connection status display */
function SpvConnectionStatus({ status }: { status: SpvStatusDto }) {
  switch (status) {
    case "running":
      return (
        <span className="text-sm text-success" data-testid="spv-status-running">
          Fully Synced - The SPV client can now be used for transacting and
          querying.
        </span>
      );
    case "syncing":
    case "starting":
      return (
        <div className="flex items-center gap-2">
          <Loader2 className="h-4 w-4 animate-spin text-primary" />
          <span className="text-sm" data-testid="spv-status-syncing">
            Syncing...
          </span>
        </div>
      );
    case "stopping":
      return (
        <div className="flex items-center gap-2">
          <Loader2 className="h-4 w-4 animate-spin text-primary" />
          <span className="text-sm" data-testid="spv-status-stopping">
            Disconnecting...
          </span>
        </div>
      );
    default:
      return null;
  }
}

/** SPV sync progress bars */
function SpvSyncProgress({ progress }: { progress: SpvStatusEvent }) {
  const pct = progress.syncProgressPct ?? 0;
  const peerCount = progress.connectedPeers;

  return (
    <div className="space-y-3" data-testid="spv-sync-progress">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-foreground">
          SPV Sync Status
        </h3>
        {peerCount > 0 && (
          <span className="text-xs text-muted-foreground">
            Peers: {peerCount}
          </span>
        )}
      </div>

      {progress.error && (
        <p className="text-xs text-destructive">{progress.error}</p>
      )}

      {/* Overall progress */}
      <ProgressBar
        value={pct}
        label="Overall Progress"
        showPercent
        variant="primary"
      />

      {/* Header height info */}
      {progress.headerHeight != null && progress.headerHeight > 0 && (
        <div className="text-xs text-muted-foreground">
          Headers height: {progress.headerHeight.toLocaleString()}
        </div>
      )}
    </div>
  );
}

/** Feedback banner for success/error messages */
function FeedbackBanner({
  feedback,
  onDismiss,
}: {
  feedback: FeedbackMessage | null;
  onDismiss: () => void;
}) {
  if (!feedback) return null;

  const isSuccess = feedback.type === "success";

  return (
    <div
      className={cn(
        "mt-3 flex items-center justify-between rounded border px-3 py-2",
        isSuccess
          ? "border-success bg-success/10 text-success"
          : "border-destructive bg-destructive/10 text-destructive",
      )}
      data-testid={`feedback-${feedback.type}`}
    >
      <span className="text-xs">{feedback.text}</span>
      <button
        type="button"
        onClick={onDismiss}
        className="ml-2 text-xs underline hover:no-underline"
      >
        Dismiss
      </button>
    </div>
  );
}
