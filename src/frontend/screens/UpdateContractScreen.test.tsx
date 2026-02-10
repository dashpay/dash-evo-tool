import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdateContractScreen } from "./UpdateContractScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { useWalletStore } from "@/stores/walletStore";
import type {
  QualifiedIdentityDto,
  IdentityKeyDto,
  ContractSummaryDto,
} from "@/bindings";

// ─── jsdom polyfills for Radix Select ───────────────────────────────

if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
}
if (!Element.prototype.setPointerCapture) {
  Element.prototype.setPointerCapture = () => {};
}
if (!Element.prototype.releasePointerCapture) {
  Element.prototype.releasePointerCapture = () => {};
}
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// ─── Centralized mock bindings ──────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

const { mockToast } = vi.hoisted(() => ({
  mockToast: {
    error: vi.fn(),
    success: vi.fn(),
    info: vi.fn(),
  },
}));

vi.mock("sonner", () => ({
  toast: mockToast,
  Toaster: () => null,
}));

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

import { commands, events } from "@/bindings";

// ─── Test Fixtures ─────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    purpose: "AUTHENTICATION",
    securityLevel: "CRITICAL",
    keyType: "ECDSA_SECP256K1",
    data: "aabb".repeat(16),
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: true,
    ...overrides,
  };
}

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Test Identity",
    balance: 10_000_000_000,
    keys: [makeKey()],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: null,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

function makeContractSummary(
  overrides: Partial<ContractSummaryDto> = {},
): ContractSummaryDto {
  return {
    id: "cc".repeat(32),
    alias: "My Contract",
    documentTypeCount: 1,
    tokenCount: 0,
    ...overrides,
  };
}

const VALID_CONTRACT_SCHEMA_JSON = {
  $format_version: "0",
  id: "cc".repeat(32),
  ownerId: "aa".repeat(32),
  version: 1,
  documentSchemas: {
    note: {
      type: "object",
      properties: { message: { type: "string" } },
    },
  },
  config: {
    $format_version: "0",
    canBeDeleted: false,
    readonly: false,
    keepsHistory: false,
  },
};

function makeWalletDto(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: "wallet-hash-1",
    usesPassword: true,
    alias: "My Wallet",
    isMain: false,
    confirmedBalance: 1000000,
    unconfirmedBalance: 0,
    totalBalance: 1000000,
    addresses: [],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [],
    passwordHint: "hint",
    ...overrides,
  };
}

// ─── Helpers ───────────────────────────────────────────────────────

function resetStores() {
  useIdentityStore.setState({
    identities: [],
    selectedIdentityId: null,
    loading: false,
    refreshingIds: new Set(),
    refreshingAll: false,
    error: null,
    sortColumn: "alias",
    sortOrder: "ascending",
    useCustomOrder: true,
  });
  useContractStore.setState({
    contracts: [],
    selectedContractId: null,
    selectedContractDetail: null,
    loading: false,
    fetching: false,
    error: null,
  });
  useWalletStore.setState({
    hdWallets: [],
    singleKeyWallets: [],
    selectedWallet: null,
    loading: false,
    refreshing: false,
    error: null,
  });
}

function setupWithIdentities(identities: QualifiedIdentityDto[]) {
  vi.mocked(commands.identityListLocal).mockResolvedValue({
    status: "ok",
    data: identities,
  });
  useIdentityStore.setState({ identities, loading: false });
}

function setupWithContracts(contracts: ContractSummaryDto[]) {
  vi.mocked(commands.contractListLocal).mockResolvedValue({
    status: "ok",
    data: contracts,
  });
  useContractStore.setState({ contracts, loading: false });
}

function fireTaskResult(payload: unknown) {
  const calls = vi.mocked(events.taskResultEvent.listen).mock.calls;
  const listener = calls[calls.length - 1]?.[0];
  act(() => {
    listener?.({ payload });
  });
}

function fireTaskError(payload: unknown) {
  const calls = vi.mocked(events.taskErrorEvent.listen).mock.calls;
  const listener = calls[calls.length - 1]?.[0];
  act(() => {
    listener?.({ payload });
  });
}

function setup() {
  const user = userEvent.setup();
  const result = render(<UpdateContractScreen />);
  return { user, ...result };
}

// ─── Tests ─────────────────────────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  vi.mocked(commands.contractUpdate).mockResolvedValue({
    status: "ok",
    data: { taskId: "task-1" },
  });
  vi.mocked(commands.contractGetById).mockResolvedValue({
    status: "ok",
    data: null,
  });
});

describe("UpdateContractScreen", () => {
  describe("rendering", () => {
    it("renders the page title", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Update Data Contract")).toBeInTheDocument();
    });

    it("renders breadcrumbs", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Contracts")).toBeInTheDocument();
      expect(
        screen.getAllByText(/Update Contract/).length,
      ).toBeGreaterThanOrEqual(2);
    });

    it("renders back button", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByRole("button", { name: /Back to Contracts/i }),
      ).toBeInTheDocument();
    });

    it("renders step headings", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
      expect(
        screen.getByText("2. Select Contract to Update"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("3. Edit the contract JSON"),
      ).toBeInTheDocument();
    });

    it("renders identity selector", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Contract Owner")).toBeInTheDocument();
    });

    it("renders contract selector", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByRole("combobox", { name: /Contract to update/i }),
      ).toBeInTheDocument();
    });

    it("renders contract JSON textarea", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByRole("textbox", { name: /Contract JSON/i }),
      ).toBeInTheDocument();
    });

    it("renders update button disabled by default", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      const btn = screen.getByRole("button", { name: /Update Contract/i });
      expect(btn).toBeDisabled();
    });

    it("renders advanced options toggle", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Advanced Options")).toBeInTheDocument();
    });

    it("renders identity balance", () => {
      const identity = makeIdentity({ balance: 10_000_000_000 });
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText(/Balance:/)).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner when identities are loading", () => {
      useIdentityStore.setState({ identities: [], loading: true });
      setup();

      expect(screen.getByText("Loading identities...")).toBeInTheDocument();
    });

    it("does not show spinner with identities loaded", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.queryByText("Loading identities..."),
      ).not.toBeInTheDocument();
    });
  });

  describe("no identities state", () => {
    it("shows warning when no identities are loaded", async () => {
      setupWithIdentities([]);
      setup();

      await waitFor(() => {
        expect(
          screen.getByText(/No identities loaded/),
        ).toBeInTheDocument();
      });
    });
  });

  describe("no critical keys warning", () => {
    it("shows warning when identity has no critical auth keys", () => {
      const identity = makeIdentity({
        keys: [makeKey({ securityLevel: "HIGH" })],
      });
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByText(/No critical authentication keys available/),
      ).toBeInTheDocument();
    });

    it("does not show warning when identity has critical auth keys", () => {
      const identity = makeIdentity({
        keys: [makeKey({ securityLevel: "CRITICAL" })],
      });
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.queryByText(/No critical authentication keys available/),
      ).not.toBeInTheDocument();
    });
  });

  describe("contract selector", () => {
    it("shows no contracts message when no contracts available", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setupWithContracts([]);
      setup();

      expect(
        screen.getByText(/No user contracts found/),
      ).toBeInTheDocument();
    });

    it("excludes system contracts from the selector", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setupWithContracts([
        makeContractSummary({ id: "aa".repeat(32), alias: "My Contract" }),
        makeContractSummary({ id: "bb".repeat(32), alias: "dpns" }),
        makeContractSummary({ id: "cc".repeat(32), alias: "dashpay" }),
        makeContractSummary({
          id: "dd".repeat(32),
          alias: "keyword_search",
        }),
        makeContractSummary({
          id: "ee".repeat(32),
          alias: "token_history",
        }),
        makeContractSummary({
          id: "ff".repeat(32),
          alias: "withdrawals",
        }),
      ]);
      setup();

      // The message about no user contracts should NOT show since we have one
      expect(
        screen.queryByText(/No user contracts found/),
      ).not.toBeInTheDocument();
    });

    it("loads contract JSON when contract is selected", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: "aa".repeat(32),
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Click the contract selector trigger
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);

      // Select the contract
      const option = await screen.findByText("My Contract");
      await user.click(option);

      // Wait for contract JSON to load into textarea
      await waitFor(() => {
        const textarea = screen.getByRole("textbox", {
          name: /Contract JSON/i,
        }) as HTMLTextAreaElement;
        expect(textarea.value).toContain("documentSchemas");
      });
    });
  });

  describe("JSON parsing", () => {
    it("shows parse error for invalid JSON", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: "aa".repeat(32),
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract to enable textarea
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        const textarea = screen.getByRole("textbox", {
          name: /Contract JSON/i,
        }) as HTMLTextAreaElement;
        expect(textarea.value).toContain("documentSchemas");
      });

      const textarea = screen.getByRole("textbox", {
        name: /Contract JSON/i,
      });
      await user.clear(textarea);
      await user.type(textarea, "{{invalid json");

      expect(screen.getByText(/Invalid JSON/)).toBeInTheDocument();
    });

    it("shows parse error for non-object JSON", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: "aa".repeat(32),
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract to enable textarea
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        const textarea = screen.getByRole("textbox", {
          name: /Contract JSON/i,
        }) as HTMLTextAreaElement;
        expect(textarea.value).toContain("documentSchemas");
      });

      const textarea = screen.getByRole("textbox", {
        name: /Contract JSON/i,
      });
      await user.clear(textarea);
      // Use paste instead of type since `[` is special in userEvent.type
      await user.click(textarea);
      await user.paste("[1, 2, 3]");

      expect(
        screen.getByText("Contract JSON must be an object."),
      ).toBeInTheDocument();
    });

    it("shows fee estimation for valid contract JSON", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: "aa".repeat(32),
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(screen.getByText(/Estimated Fee:/)).toBeInTheDocument();
      });
    });

    it("enables update button with valid contract and identity", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: "aa".repeat(32),
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        const btn = screen.getByRole("button", {
          name: /Update Contract/i,
        });
        expect(btn).toBeEnabled();
      });
    });
  });

  describe("advanced options", () => {
    it("shows key selector when advanced options is toggled", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({ keyId: 0, securityLevel: "CRITICAL" }),
          makeKey({ keyId: 1, securityLevel: "CRITICAL" }),
        ],
      });
      setupWithIdentities([identity]);
      const { user } = setup();

      await user.click(screen.getByText("Advanced Options"));

      expect(screen.getByText("Signing Key")).toBeInTheDocument();
    });

    it("hides key selector when advanced options is toggled off", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      await user.click(screen.getByText("Advanced Options"));
      expect(screen.getByText("Signing Key")).toBeInTheDocument();

      await user.click(screen.getByText("Advanced Options"));
      expect(screen.queryByText("Signing Key")).not.toBeInTheDocument();
    });
  });

  describe("contract update dispatch", () => {
    it("dispatches contractUpdate IPC call", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        const btn = screen.getByRole("button", {
          name: /Update Contract/i,
        });
        expect(btn).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      expect(commands.contractUpdate).toHaveBeenCalledWith({
        contractJson: expect.objectContaining({
          documentSchemas: expect.any(Object),
          ownerId: identity.id,
        }),
        identityId: identity.id,
        keyId: 0,
      });
    });

    it("shows broadcasting state after dispatch", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      expect(
        screen.getByText(/Broadcasting contract update/),
      ).toBeInTheDocument();
    });

    it("shows success screen on task result event", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskResult({
        taskId: "task-1",
        resultType: "Contract",
        payload: null,
      });

      expect(
        screen.getByText("Contract Updated Successfully"),
      ).toBeInTheDocument();
      expect(mockToast.success).toHaveBeenCalledWith(
        "Contract updated successfully!",
      );
    });

    it("shows error on task error event", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskError({
        taskId: "task-1",
        message: "Insufficient balance",
      });

      expect(screen.getByText("Update Failed")).toBeInTheDocument();
      expect(screen.getByText("Insufficient balance")).toBeInTheDocument();
    });

    it("shows error on IPC dispatch error", async () => {
      vi.mocked(commands.contractUpdate).mockResolvedValue({
        status: "error",
        error: "IPC error occurred",
      });
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      expect(screen.getByText("Update Failed")).toBeInTheDocument();
      expect(screen.getByText("IPC error occurred")).toBeInTheDocument();
    });

    it("shows error on IPC exception", async () => {
      vi.mocked(commands.contractUpdate).mockRejectedValue(
        new Error("Connection lost"),
      );
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      // Select contract
      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      expect(screen.getByText("Update Failed")).toBeInTheDocument();
      expect(screen.getByText("Connection lost")).toBeInTheDocument();
    });

    it("ignores task results with wrong taskId", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskResult({
        taskId: "wrong-task",
        resultType: "Contract",
        payload: null,
      });

      // Should still be in broadcasting state
      expect(
        screen.getByText(/Broadcasting contract update/),
      ).toBeInTheDocument();
    });

    it("ignores non-Contract result types", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskResult({
        taskId: "task-1",
        resultType: "Identity",
        payload: null,
      });

      expect(
        screen.getByText(/Broadcasting contract update/),
      ).toBeInTheDocument();
    });
  });

  describe("error dismissal", () => {
    it("returns to input on error dismissal", async () => {
      vi.mocked(commands.contractUpdate).mockResolvedValue({
        status: "error",
        error: "Test error",
      });
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      expect(screen.getByText("Update Failed")).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: /Dismiss/i }));

      expect(screen.queryByText("Update Failed")).not.toBeInTheDocument();
      expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
    });
  });

  describe("success screen", () => {
    it("renders success message and action buttons", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskResult({
        taskId: "task-1",
        resultType: "Contract",
        payload: null,
      });

      expect(
        screen.getByText("Contract Updated Successfully"),
      ).toBeInTheDocument();
      expect(
        screen.getAllByRole("button", { name: /Back to Contracts/i }).length,
      ).toBeGreaterThanOrEqual(1);
      expect(
        screen.getByRole("button", { name: /Update Another Contract/i }),
      ).toBeInTheDocument();
    });

    it("navigates back on Back to Contracts click", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskResult({
        taskId: "task-1",
        resultType: "Contract",
        payload: null,
      });

      // Click back (in success screen, not header)
      const backButtons = screen.getAllByRole("button", {
        name: /Back to Contracts/i,
      });
      await user.click(backButtons[backButtons.length - 1]);

      expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
    });

    it("resets form on Update Another Contract click", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      const { user } = setup();

      const trigger = screen.getByRole("combobox", {
        name: /Contract to update/i,
      });
      await user.click(trigger);
      const option = await screen.findByText("My Contract");
      await user.click(option);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Update Contract/i }),
        ).toBeEnabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Update Contract/i }),
      );

      fireTaskResult({
        taskId: "task-1",
        resultType: "Contract",
        payload: null,
      });

      await user.click(
        screen.getByRole("button", { name: /Update Another Contract/i }),
      );

      // Back to input state
      expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
      // Contract JSON should be cleared
      const textarea = screen.getByRole("textbox", {
        name: /Contract JSON/i,
      });
      expect(textarea).toHaveValue("");
    });
  });

  describe("navigation", () => {
    it("navigates back on back button click", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      await user.click(
        screen.getByRole("button", { name: /Back to Contracts/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
    });
  });

  describe("wallet lock", () => {
    it("shows wallet locked warning when wallet needs unlock", () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      useWalletStore.setState({
        hdWallets: [makeWalletDto() as never],
        singleKeyWallets: [],
      });
      setup();

      expect(
        screen.getByText(/Wallet is locked/),
      ).toBeInTheDocument();
    });

    it("disables update button when wallet is locked", () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      useWalletStore.setState({
        hdWallets: [makeWalletDto() as never],
        singleKeyWallets: [],
      });
      setup();

      const btn = screen.getByRole("button", { name: /Update Contract/i });
      expect(btn).toBeDisabled();
    });

    it("shows unlock button in wallet locked warning", () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      useWalletStore.setState({
        hdWallets: [makeWalletDto() as never],
        singleKeyWallets: [],
      });
      setup();

      expect(
        screen.getByRole("button", { name: /Unlock Wallet/i }),
      ).toBeInTheDocument();
    });

    it("does not show locked warning when wallet has no password", () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      useWalletStore.setState({
        hdWallets: [makeWalletDto({ usesPassword: false }) as never],
        singleKeyWallets: [],
      });
      setup();

      expect(
        screen.queryByText(/Wallet is locked/),
      ).not.toBeInTheDocument();
    });

    it("does not show locked warning without associated wallet", () => {
      const identity = makeIdentity({ associatedWalletHashes: [] });
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.queryByText(/Wallet is locked/),
      ).not.toBeInTheDocument();
    });
  });

  describe("event subscription", () => {
    it("subscribes to task result and error events", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      await waitFor(() => {
        expect(events.taskResultEvent.listen).toHaveBeenCalled();
        expect(events.taskErrorEvent.listen).toHaveBeenCalled();
      });
    });
  });

  describe("key auto-selection", () => {
    it("auto-selects CRITICAL key", () => {
      const identity = makeIdentity({
        keys: [
          makeKey({ keyId: 0, securityLevel: "HIGH" }),
          makeKey({ keyId: 1, securityLevel: "CRITICAL" }),
        ],
      });
      setupWithIdentities([identity]);
      const contract = makeContractSummary();
      setupWithContracts([contract]);
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: {
          id: contract.id,
          ownerId: identity.id,
          alias: "My Contract",
          version: 1,
          documentTypeNames: ["note"],
          tokenCount: 0,
          schemaJson: VALID_CONTRACT_SCHEMA_JSON,
        },
      });
      setup();

      // If we had CRITICAL key auto-selected, the update button should
      // still be disabled (no contract selected), but no key warning
      expect(
        screen.queryByText(/No critical authentication keys available/),
      ).not.toBeInTheDocument();
    });

    it("shows disabled state with no eligible keys", () => {
      const identity = makeIdentity({
        keys: [makeKey({ keyId: 0, securityLevel: "HIGH" })],
      });
      setupWithIdentities([identity]);
      setup();

      // Should show critical key warning
      expect(
        screen.getByText(/No critical authentication keys available/),
      ).toBeInTheDocument();

      // Update button should be disabled
      const btn = screen.getByRole("button", { name: /Update Contract/i });
      expect(btn).toBeDisabled();
    });
  });

  describe("textarea disabled state", () => {
    it("textarea is disabled when no contract is selected", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      const textarea = screen.getByRole("textbox", {
        name: /Contract JSON/i,
      });
      expect(textarea).toBeDisabled();
    });
  });
});
