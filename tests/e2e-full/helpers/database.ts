/**
 * Database helpers for E2E tests.
 *
 * Provides utilities for seeding the test database with known state
 * before tests and cleaning up after.
 *
 * The database path depends on the platform and app configuration.
 * In Docker E2E, the app uses a predictable path.
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { execSync } from "child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

/** Database location in the Docker E2E environment */
const DB_PATHS = {
  linux: path.join(
    process.env.HOME || "/root",
    ".config/dash-evo-tool/dash-evo-tool.db"
  ),
  darwin: path.join(
    process.env.HOME || "/Users/dev",
    "Library/Application Support/Dash-Evo-Tool/dash-evo-tool.db"
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
 * Run a SQL file against the database using sqlite3 CLI.
 * Requires sqlite3 to be installed (it is in the Docker image).
 */
export function runSqlFile(sqlFilePath: string): void {
  const dbPath = getDatabasePath();
  const resolvedSql = path.resolve(sqlFilePath);

  if (!fs.existsSync(resolvedSql)) {
    throw new Error(`SQL file not found: ${resolvedSql}`);
  }

  try {
    execSync(`sqlite3 "${dbPath}" < "${resolvedSql}"`, {
      stdio: "pipe",
      timeout: 10_000,
    });
  } catch (err) {
    const error = err as { stderr?: Buffer };
    throw new Error(
      `Failed to run SQL file ${resolvedSql}: ${error.stderr?.toString() || String(err)}`
    );
  }
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
 * Seed the database with test fixtures from the seed-data.sql file.
 */
export function seedTestData(): void {
  const seedFile = path.resolve(__dirname, "../fixtures/seed-data.sql");
  runSqlFile(seedFile);
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
 * Full test database setup: clear data, reset settings, seed fixtures.
 */
export function setupTestDatabase(network = "testnet"): void {
  if (!databaseExists()) {
    console.warn(
      `Database not found at ${getDatabasePath()}. ` +
        `The app may need to run once to create it.`
    );
    return;
  }
  clearAllData();
  resetSettings(network);
  seedTestData();
}

/**
 * Tear down test database: clear all user data.
 */
export function teardownTestDatabase(): void {
  if (databaseExists()) {
    clearAllData();
  }
}
