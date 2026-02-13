/**
 * Database helpers for E2E tests.
 *
 * Provides utilities for querying/resetting the app database.
 * Real testnet E2E tests do NOT seed fake data — wallet state comes from
 * real SPV sync and IPC. These helpers are for pre-test cleanup only.
 *
 * The database path depends on the platform and app configuration.
 * In Docker E2E, the app uses a predictable path.
 */

import fs from "fs";
import path from "path";
import { execSync } from "child_process";

/** Database location in the Docker E2E environment.
 *  The app uses `directories::ProjectDirs::from("", "", "Dash-Evo-Tool")`.
 *  On Linux this resolves to `~/.config/Dash-Evo-Tool/`.
 *  The database file is `data.db` (see src-tauri/src/state.rs).
 */
const DB_PATHS = {
  linux: path.join(
    process.env.HOME || "/root",
    ".config/Dash-Evo-Tool/data.db"
  ),
  darwin: path.join(
    process.env.HOME || "/Users/dev",
    "Library/Application Support/Dash-Evo-Tool/data.db"
  ),
};

/**
 * Get the database path for the current platform.
 */
export function getDatabasePath(): string {
  const platform = process.platform;
  if (platform === "linux") return DB_PATHS.linux;
  if (platform === "darwin") return DB_PATHS.darwin;
  throw new Error(`Unsupported platform for E2E tests: ${platform}`);
}

/**
 * Check if the database file exists.
 */
export function databaseExists(): boolean {
  return fs.existsSync(getDatabasePath());
}

/**
 * Run raw SQL against the database.
 */
export function runSql(sql: string): void {
  const dbPath = getDatabasePath();
  try {
    execSync(`sqlite3 "${dbPath}" "${sql.replace(/"/g, '\\"')}"`, {
      stdio: "pipe",
      timeout: 10_000,
    });
  } catch (err) {
    const error = err as { stderr?: Buffer };
    throw new Error(
      `Failed to run SQL: ${error.stderr?.toString() || String(err)}`
    );
  }
}

/**
 * Query the database and return results as a string.
 */
export function querySql(sql: string): string {
  const dbPath = getDatabasePath();
  try {
    const result = execSync(
      `sqlite3 -json "${dbPath}" "${sql.replace(/"/g, '\\"')}"`,
      {
        stdio: "pipe",
        timeout: 10_000,
      }
    );
    return result.toString().trim();
  } catch (err) {
    const error = err as { stderr?: Buffer };
    throw new Error(
      `Failed to query SQL: ${error.stderr?.toString() || String(err)}`
    );
  }
}

/**
 * Clear all user data from the database while preserving the schema.
 * Useful for resetting between test suites.
 */
export function clearAllData(): void {
  const tables = [
    "dashpay_address_mappings",
    "dashpay_contact_address_indices",
    "dashpay_payments",
    "dashpay_contact_requests",
    "dashpay_contacts",
    "dashpay_profiles",
    "contact_private_info",
    "scheduled_votes",
    "identity_token_balances",
    "token_order",
    "token",
    "proof_log",
    "contestant",
    "contested_name",
    "asset_lock_transaction",
    "utxos",
    "top_up",
    "identity_order",
    "identity",
    "platform_address_balances",
    "wallet_transactions",
    "wallet_addresses",
    "wallet",
    "contract",
  ];

  // Delete in order respecting foreign key constraints
  const sql = tables.map((t) => `DELETE FROM ${t};`).join(" ");
  runSql(sql);
}

/**
 * Reset the settings table to defaults for testing.
 * Ensures predictable test state.
 */
export function resetSettings(network = "testnet"): void {
  runSql(`
    DELETE FROM settings;
    INSERT INTO settings (
      id, network, start_root_screen, database_version,
      onboarding_completed, theme_preference, disable_zmq,
      core_backend_mode
    ) VALUES (
      1, '${network}', 0, 1,
      1, 'System', 1,
      1
    );
  `);
}

/**
 * Ensure the database starts clean before a test run.
 * Clears user data and resets settings; does NOT seed fake data.
 */
export function prepareCleanDatabase(network = "testnet"): void {
  if (!databaseExists()) {
    console.warn(
      `Database not found at ${getDatabasePath()}. ` +
        `The app may need to run once to create it.`
    );
    return;
  }
  clearAllData();
  resetSettings(network);
}
