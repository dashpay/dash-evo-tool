/**
 * 00-smoke — Proves the IPC bridge is operational.
 *
 * This is the first spec in the numbered sequence. It verifies:
 * 1. The Tauri app launches and renders UI.
 * 2. `window.__TAURI_INTERNALS__.invoke()` is reachable from WebdriverIO.
 * 3. A simple IPC command (`get_app_version`) returns a sane value.
 * 4. A richer IPC command (`get_network_info`) returns typed data.
 *
 * If this spec fails, none of the later specs will work — it's the
 * canary that tells you whether the backend bridge is alive.
 */

import { waitForAppReady } from "../helpers/tauri.js";
import { invoke, isBridgeAvailable } from "../helpers/ipc.js";

describe("IPC Smoke Test", () => {
  it("should launch the Tauri app and render the UI", async () => {
    await waitForAppReady();

    const title = await browser.getTitle();
    expect(title).toBe("Dash Evo Tool");
  });

  it("should have the Tauri IPC bridge available", async () => {
    const available = await isBridgeAvailable();
    expect(available).toBe(true);
  });

  it("should return the app version via IPC", async () => {
    const version = await invoke<string>("get_app_version");

    // Version should be a semver-ish string (e.g. "0.1.0")
    expect(typeof version).toBe("string");
    expect(version.length).toBeGreaterThan(0);
    expect(version).toMatch(/^\d+\.\d+/);
  });

  it("should return network info via IPC", async () => {
    const info = await invoke<{
      activeNetwork: string;
      availableNetworks: string[];
    }>("get_network_info");

    expect(info).toBeDefined();
    expect(typeof info.activeNetwork).toBe("string");
    expect(Array.isArray(info.availableNetworks)).toBe(true);
    expect(info.availableNetworks.length).toBeGreaterThan(0);
  });

  it("should return settings via IPC", async () => {
    const result = await invoke<{
      status: string;
      data?: { network: string };
    }>("settings_get");

    // settings_get wraps in Result<T, String>
    expect(result).toBeDefined();
  });

  it("should return SPV status array via IPC", async () => {
    const statuses = await invoke<
      Array<{ network: string; status: string }>
    >("get_spv_status");

    expect(Array.isArray(statuses)).toBe(true);
  });
});
