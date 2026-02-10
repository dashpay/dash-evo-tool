/**
 * Browser-side Mock IPC for Playwright Integration Tests
 *
 * This module is loaded conditionally when VITE_E2E_MOCK=true.
 * It uses @tauri-apps/api/mocks to intercept all IPC calls
 * and provides a window.__E2E_MOCK_IPC__ interface for Playwright
 * to configure mock responses via page.evaluate().
 *
 * IMPORTANT: Mock responses are what TAURI_INVOKE (the raw invoke function)
 * would return. The bindings.ts wrapper adds the Result { status, data }
 * envelope for commands that use the try/catch pattern. So for a command
 * like wallet_list_all, return the raw inner data:
 *   { hdWallets: [], singleKeyWallets: [], selected: null }
 * NOT:
 *   { status: "ok", data: { hdWallets: [], ... } }
 *
 * Usage from Playwright:
 *
 *   await mockIPC.setHandler("wallet_list_all", {
 *     hdWallets: [walletData],
 *     singleKeyWallets: [],
 *     selected: null,
 *   });
 */

import { mockIPC, mockWindows, clearMocks } from "@tauri-apps/api/mocks";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface MockIPCConfig {
  /** Per-command response overrides. Key = snake_case command name. */
  handlers: Record<string, unknown>;
  /** Call history: command name → array of args. */
  callHistory: Record<string, unknown[]>;
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

const config: MockIPCConfig = {
  handlers: {},
  callHistory: {},
};

// ---------------------------------------------------------------------------
// Default responses for all commands
//
// These return what TAURI_INVOKE would return — the raw Rust return value.
// bindings.ts adds the Result { status: "ok", data } wrapper where needed.
//
// For dispatch-style commands (backend tasks), just return { taskId: "..." }.
// For Result<T, String> commands, return the raw T value.
// For direct-return commands, return the full expected value.
// ---------------------------------------------------------------------------

const DISPATCH = { taskId: "mock-task-id" };

const DEFAULT_RESPONSES: Record<string, unknown> = {
  // -- General (direct return) --
  greet: { message: "Hello, Mock!" },
  get_app_version: "0.0.0-test",
  get_network_info: {
    network: "Testnet",
    coreVersion: "21.0.0",
    platformVersion: "1.0.0",
    connected: true,
  },
  switch_network: null,
  get_spv_status: [],

  // -- Identity (Result-wrapped → return raw inner data) --
  identity_load: DISPATCH,
  identity_search_by_dpns_name: DISPATCH,
  identity_search_from_wallet: DISPATCH,
  identity_search_up_to_index: DISPATCH,
  identity_register_dpns_name: DISPATCH,
  identity_refresh: DISPATCH,
  identity_refresh_dpns_names: DISPATCH, // direct return
  identity_withdraw: DISPATCH,
  identity_transfer: DISPATCH,
  identity_add_key: DISPATCH,
  identity_disable_keys: DISPATCH,
  identity_replace_key: DISPATCH,
  identity_register: DISPATCH,
  identity_top_up: DISPATCH,
  identity_top_up_from_platform_addresses: DISPATCH,
  identity_transfer_to_addresses: DISPATCH,
  identity_list_local: [],
  identity_list_user: [],
  identity_list_voting: [],
  identity_get_by_id: null,
  identity_set_alias: null,
  identity_get_alias: null,
  identity_load_order: [],
  identity_save_order: null,
  identity_delete: null,
  identity_list_summaries: [],
  identity_local_dpns_names: [],
  identity_sign_message: "",

  // -- Core --
  core_get_best_chain_lock: DISPATCH, // direct return
  core_get_best_chain_locks: DISPATCH, // direct return
  core_refresh_wallet_info: DISPATCH,
  core_refresh_single_key_wallet_info: DISPATCH,
  core_start_dash_qt: DISPATCH, // direct return
  core_create_registration_asset_lock: DISPATCH,
  core_create_top_up_asset_lock: DISPATCH,
  core_send_wallet_payment: DISPATCH,
  core_send_single_key_wallet_payment: DISPATCH,
  core_recover_asset_locks: DISPATCH,

  // -- Wallet --
  wallet_generate_receive_address: DISPATCH,
  wallet_fetch_platform_address_balances: DISPATCH,
  wallet_transfer_platform_credits: DISPATCH,
  wallet_withdraw_from_platform_address: DISPATCH,
  wallet_fund_platform_address_from_utxos: DISPATCH,
  wallet_fund_platform_from_asset_lock: DISPATCH,
  wallet_create: {
    seedHash: "mock-seed-hash",
    alias: "Mock Wallet",
    identityRegistrations: [],
    accounts: [],
    utxos: [],
    assetLocks: [],
  },
  wallet_import_mnemonic: {
    seedHash: "mock-seed-hash",
    alias: "Imported Wallet",
    identityRegistrations: [],
    accounts: [],
    utxos: [],
    assetLocks: [],
  },
  wallet_import_private_key: {
    keyHash: "mock-key-hash",
    alias: "Imported Key",
    address: "yMockAddress123",
    balanceSatoshis: 0,
    utxos: [],
  },
  wallet_list_all: {
    hdWallets: [],
    singleKeyWallets: [],
    selected: null,
  },
  wallet_get_hd: null,
  wallet_get_single_key: null,
  wallet_select: null,
  wallet_set_alias: null,
  wallet_set_single_key_alias: null,
  wallet_remove: null,
  wallet_remove_single_key: null,
  wallet_start_spv: null,
  wallet_stop_spv: undefined, // void
  wallet_clear_spv_data: null,
  wallet_bootstrap_addresses: null,
  wallet_notify_unlocked: null,
  wallet_notify_locked: null,
  wallet_get_private_key: "",

  // -- Contract --
  contract_fetch: DISPATCH,
  contract_fetch_with_descriptions: DISPATCH,
  contract_fetch_active_group_actions: DISPATCH,
  contract_register: DISPATCH,
  contract_update: DISPATCH,
  contract_save: DISPATCH,
  contract_remove: null,
  contract_list_local: [],
  contract_get_by_id: null,
  contract_get_by_token_id: null,
  contract_set_alias: null,

  // -- Document --
  document_broadcast: DISPATCH,
  document_delete: DISPATCH,
  document_replace: DISPATCH,
  document_transfer: DISPATCH,
  document_purchase: DISPATCH,
  document_set_price: DISPATCH,
  document_fetch: DISPATCH,
  document_fetch_page: DISPATCH,

  // -- Token --
  token_query_my_balances: DISPATCH, // direct return
  token_query_identity_balance: DISPATCH,
  token_query_frozen_identities: DISPATCH,
  token_query_descriptions_by_keyword: DISPATCH,
  token_fetch_by_contract_id: DISPATCH,
  token_fetch_by_token_id: DISPATCH,
  token_save_locally: DISPATCH,
  token_query_pricing: DISPATCH,
  token_mint: DISPATCH,
  token_transfer: DISPATCH,
  token_burn: DISPATCH,
  token_destroy_frozen_funds: DISPATCH,
  token_freeze: DISPATCH,
  token_unfreeze: DISPATCH,
  token_pause: DISPATCH,
  token_resume: DISPATCH,
  token_claim: DISPATCH,
  token_estimate_perpetual_rewards: DISPATCH,
  token_query_claims: DISPATCH,
  token_update_config: DISPATCH,
  token_purchase: DISPATCH,
  token_set_direct_purchase_price: DISPATCH,
  token_register_contract: DISPATCH,
  token_remove: null,
  token_load_order: [],
  token_save_order: null,
  token_get_minting_config: {
    allowChoosingDestination: true,
    defaultDestinationIdentityId: null,
  },

  // -- DashPay --
  dashpay_load_profile: DISPATCH,
  dashpay_update_profile: DISPATCH,
  dashpay_load_contacts: DISPATCH,
  dashpay_load_contact_requests: DISPATCH,
  dashpay_fetch_contact_profile: DISPATCH,
  dashpay_search_profiles: DISPATCH,
  dashpay_send_contact_request: DISPATCH,
  dashpay_send_contact_request_with_proof: DISPATCH,
  dashpay_accept_contact_request: DISPATCH,
  dashpay_reject_contact_request: DISPATCH,
  dashpay_load_payment_history: DISPATCH,
  dashpay_send_payment_to_contact: DISPATCH,
  dashpay_update_contact_info: DISPATCH,
  dashpay_register_addresses: DISPATCH,
  dashpay_db_load_profile: null,
  dashpay_db_save_profile: null,
  dashpay_db_load_contacts: [],
  dashpay_db_load_pending_requests: [],
  dashpay_db_load_payments: [],
  dashpay_db_load_contact_private_info: {
    nickname: "",
    notes: "",
    isHidden: false,
  },
  dashpay_db_save_contact_private_info: null,
  dashpay_db_set_contact_hidden: null,
  dashpay_db_save_avatar_bytes: null,

  // -- Contested / DPNS --
  contested_query_dpns_contests: DISPATCH, // direct return
  contested_vote_on_dpns_names: DISPATCH,
  contested_schedule_dpns_votes: DISPATCH,
  contested_cast_scheduled_vote: DISPATCH,
  contested_clear_all_scheduled_votes: DISPATCH, // direct return
  contested_clear_executed_scheduled_votes: DISPATCH, // direct return
  contested_delete_scheduled_vote: DISPATCH,
  contested_get_scheduled_votes: [],

  // -- Parsers --
  parse_data_contract: { json: "{}", id: "" },
  parse_document: { json: "{}" },

  // -- Platform Info (direct return) --
  platform_current_epoch_info: DISPATCH,
  platform_total_credits: DISPATCH,
  platform_version_voting_state: DISPATCH,
  platform_validator_set_info: DISPATCH,
  platform_withdrawals_in_queue: DISPATCH,
  platform_recently_completed_withdrawals: DISPATCH,
  platform_basic_info: DISPATCH,
  platform_fetch_address_balance: DISPATCH, // direct return

  // -- System (direct return) --
  system_wipe_platform_data: DISPATCH,
  system_update_theme: DISPATCH,

  // -- Masternode --
  mnlist_fetch_diff: DISPATCH,
  mnlist_fetch_qr_info: DISPATCH,
  mnlist_fetch_qr_info_with_dmls: DISPATCH,
  mnlist_fetch_chain_locks: DISPATCH, // direct return
  mnlist_fetch_diffs_chain: DISPATCH,

  // -- GroveSTARK --
  grovestark_generate_proof: DISPATCH,
  grovestark_verify_proof: DISPATCH,

  // -- Broadcast --
  broadcast_state_transition: DISPATCH,

  // -- Settings --
  settings_get: {
    theme: "Dark",
    developerMode: false,
    disableZmq: false,
    onboardingCompleted: true,
    showEvonodeTools: false,
    userMode: "Basic",
    closeDashQtOnExit: false,
    autoStartSpv: false,
  },
  settings_update_password: null,
  settings_update_dash_core: null,
  settings_update_disable_zmq: null,
  settings_update_onboarding_completed: null,
  settings_update_show_evonode_tools: null,
  settings_update_user_mode: null,
  settings_update_close_dash_qt_on_exit: null,
  settings_update_auto_start_spv: null,
  settings_get_auto_start_spv: false,

  // -- Context --
  context_is_developer_mode: false, // direct return
  context_enable_developer_mode: undefined, // void, direct return
  context_get_fee_multiplier: 1.0, // direct return
  context_set_fee_multiplier: undefined, // void, direct return
  context_get_network: "Testnet", // direct return
  context_get_core_backend_mode: "Rpc", // direct return
  context_set_core_backend_mode: null,
};

// ---------------------------------------------------------------------------
// Initialize mock IPC
// ---------------------------------------------------------------------------

export function initE2EMockIPC() {
  // Mock windows first (required for @tauri-apps/api to work)
  mockWindows("main");

  // Apply any pre-configured overrides (set via page.addInitScript before load)
  const preOverrides = (window as unknown as Record<string, unknown>).__E2E_MOCK_OVERRIDES__ as Record<string, unknown> | undefined;
  if (preOverrides) {
    Object.assign(config.handlers, preOverrides);
  }

  // Set up the IPC mock handler with event support enabled
  mockIPC(
    (cmd: string, args?: Record<string, unknown>) => {
      // Record call history (skip internal plugin commands)
      if (!cmd.startsWith("plugin:")) {
        if (!config.callHistory[cmd]) {
          config.callHistory[cmd] = [];
        }
        config.callHistory[cmd].push(args ?? {});
      }

      // Check for custom handler first, then fall back to defaults
      if (cmd in config.handlers) {
        const handler = config.handlers[cmd];
        return typeof handler === "function"
          ? (handler as (args: unknown) => unknown)(args)
          : handler;
      }

      if (cmd in DEFAULT_RESPONSES) {
        return DEFAULT_RESPONSES[cmd];
      }

      // Unknown command — log and return null
      if (!cmd.startsWith("plugin:")) {
        console.warn(`[E2E Mock IPC] No handler for command: ${cmd}`, args);
      }
      return null;
    },
    { shouldMockEvents: true },
  );

  // Expose the control interface for Playwright
  const api = {
    /**
     * Set a mock response for a specific command.
     * The command name should be snake_case (matching Tauri's internal naming).
     *
     * Return the RAW value that Tauri invoke would return.
     * For Result-wrapped commands, return the inner T value (not { status, data }).
     */
    setHandler(cmd: string, response: unknown) {
      config.handlers[cmd] = response;
    },

    /**
     * Set multiple mock handlers at once.
     */
    setHandlers(handlers: Record<string, unknown>) {
      Object.assign(config.handlers, handlers);
    },

    /**
     * Get the call history for a specific command.
     */
    getCallHistory(cmd: string): unknown[] {
      return config.callHistory[cmd] ?? [];
    },

    /**
     * Get all call history.
     */
    getAllCallHistory(): Record<string, unknown[]> {
      return { ...config.callHistory };
    },

    /**
     * Clear call history for all or a specific command.
     */
    clearCallHistory(cmd?: string) {
      if (cmd) {
        delete config.callHistory[cmd];
      } else {
        for (const key of Object.keys(config.callHistory)) {
          delete config.callHistory[key];
        }
      }
    },

    /**
     * Reset all custom handlers and call history.
     */
    reset() {
      for (const key of Object.keys(config.handlers)) {
        delete config.handlers[key];
      }
      for (const key of Object.keys(config.callHistory)) {
        delete config.callHistory[key];
      }
    },

    /**
     * Emit a mock event to all registered listeners.
     * Uses the Tauri event API mock to emit events. Event names use
     * kebab-case (e.g., "task-result-event", "wallet-updated-event").
     */
    async emitEvent(eventName: string, payload: unknown) {
      // The @tauri-apps/api/mocks event system handles this internally
      // when shouldMockEvents: true. We use the emit API directly.
      const { emit } = await import("@tauri-apps/api/event");
      await emit(eventName, payload);
    },

    /**
     * Check if mock IPC is initialized.
     */
    isInitialized: true as const,
  };

  // Expose on window for Playwright access
  (window as unknown as Record<string, unknown>).__E2E_MOCK_IPC__ = api;

  console.log("[E2E Mock IPC] Initialized with", Object.keys(DEFAULT_RESPONSES).length, "default handlers");
}

// ---------------------------------------------------------------------------
// Cleanup (if needed)
// ---------------------------------------------------------------------------

export function cleanupE2EMockIPC() {
  clearMocks();
  delete (window as unknown as Record<string, unknown>).__E2E_MOCK_IPC__;
}

// ---------------------------------------------------------------------------
// TypeScript declaration for window augmentation
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    __E2E_MOCK_IPC__?: {
      setHandler(cmd: string, response: unknown): void;
      setHandlers(handlers: Record<string, unknown>): void;
      getCallHistory(cmd: string): unknown[];
      getAllCallHistory(): Record<string, unknown[]>;
      clearCallHistory(cmd?: string): void;
      reset(): void;
      emitEvent(eventName: string, payload: unknown): Promise<void>;
      isInitialized: true;
    };
  }
}
