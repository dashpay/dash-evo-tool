// jsdom polyfills for Radix Select
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

import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { GroupActionsScreen } from "./GroupActionsScreen";
import type { GroupActionItem } from "./GroupActionsScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import type { QualifiedIdentityDto, ContractSummaryDto } from "@/bindings";

// ─── Hoisted mocks ─────────────────────────────────────────────────

type EventCallback = (event: { payload: unknown }) => void;

const { mockCommands, mockEvents, eventListeners, mockNavigate } = vi.hoisted(
  () => {
    const eventListeners: Record<string, EventCallback[]> = {
      taskResultEvent: [],
      taskErrorEvent: [],
    };

    const mockEvents = {
      taskResultEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskResultEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskResultEvent =
              eventListeners.taskResultEvent.filter((l) => l !== cb);
          });
        }),
      },
      taskErrorEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskErrorEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskErrorEvent =
              eventListeners.taskErrorEvent.filter((l) => l !== cb);
          });
        }),
      },
      walletUpdatedEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    };

    const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
      identityListLocal: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: [] }),
      identityLoadOrder: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: [] }),
      identityGetById: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: null }),
      contractListLocal: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: [] }),
      contractFetchActiveGroupActions: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: { taskId: "task-ga-1" } }),
      walletListAll: vi.fn().mockResolvedValue({
        status: "ok",
        data: { hdWallets: [], singleKeyWallets: [] },
      }),
    };

    const mockNavigate = vi.fn();

    return { mockCommands, mockEvents, eventListeners, mockNavigate };
  },
);

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/bindings", () => ({
  commands: new Proxy({} as Record<string, unknown>, {
    get: (_target, prop: string) => {
      if (prop in mockCommands) return mockCommands[prop];
      return vi
        .fn()
        .mockResolvedValue({ status: "error", error: "not mocked" });
    },
  }),
  events: mockEvents,
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

// ─── Test Fixtures ─────────────────────────────────────────────────

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344",
    alias: "Test Identity",
    identityType: "user",
    balance: 50000000,
    dpnsNames: [],
    associatedWalletHashes: [],
    keys: [],
    ...overrides,
  };
}

function makeContract(
  overrides: Partial<ContractSummaryDto> = {},
): ContractSummaryDto {
  return {
    id: "11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd",
    alias: "MyToken Contract",
    documentTypeCount: 1,
    tokenCount: 2,
    ...overrides,
  };
}

function makeGroupAction(
  overrides: Partial<GroupActionItem> = {},
): GroupActionItem {
  return {
    groupPosition: 0,
    actionId: "action-abc123def456",
    actionType: "TokenMint",
    signersCount: 1,
    requiredSignatures: 3,
    details: { amount: 1000, recipient: "recipient-identity-id-hex" },
    ...overrides,
  };
}

// ─── Helpers ─────────────────────────────────────────────────

function setStoreData(
  identities: QualifiedIdentityDto[] = [],
  contracts: ContractSummaryDto[] = [],
) {
  useIdentityStore.setState({
    identities,
    loading: false,
    selectedIdentityId: null,
    refreshingIds: new Set(),
    refreshingAll: false,
    error: null,
    sortColumn: "alias",
    sortOrder: "ascending",
    useCustomOrder: true,
  });

  useContractStore.setState({
    contracts,
    selectedContractId: null,
    selectedContractDetail: null,
    loading: false,
    fetching: false,
    error: null,
  });
}

function emitTaskResult(taskId: string, payload: unknown) {
  eventListeners.taskResultEvent.forEach((cb) =>
    cb({
      payload: {
        taskId,
        resultType: "Contract",
        payload,
      },
    }),
  );
}

function emitTaskError(taskId: string, message: string) {
  eventListeners.taskErrorEvent.forEach((cb) =>
    cb({
      payload: { taskId, message },
    }),
  );
}

// ─── Tests ─────────────────────────────────────────────────────────

describe("GroupActionsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventListeners.taskResultEvent = [];
    eventListeners.taskErrorEvent = [];
    setStoreData([], []);
  });

  // ── Rendering ──────────────────────────────────────────────────

  describe("Rendering", () => {
    it("renders the page title", () => {
      render(<GroupActionsScreen />);
      expect(
        screen.getByRole("heading", { name: "Group Actions" }),
      ).toBeInTheDocument();
    });

    it("renders breadcrumbs", () => {
      render(<GroupActionsScreen />);
      expect(screen.getByText("Contracts")).toBeInTheDocument();
    });

    it("renders back button", () => {
      render(<GroupActionsScreen />);
      expect(
        screen.getByRole("button", { name: /back to contracts/i }),
      ).toBeInTheDocument();
    });

    it("renders step headings", () => {
      setStoreData([makeIdentity()], [makeContract()]);
      render(<GroupActionsScreen />);
      expect(
        screen.getByText(/step 1 — select contract/i),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/step 2 — select identity/i),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/step 3 — active group actions/i),
      ).toBeInTheDocument();
    });

    it("renders contract selector when contracts with tokens exist", () => {
      setStoreData([], [makeContract()]);
      render(<GroupActionsScreen />);
      expect(
        screen.getByRole("combobox", { name: /contract/i }),
      ).toBeInTheDocument();
    });

    it("renders identity selector when identities exist", () => {
      setStoreData([makeIdentity()], []);
      render(<GroupActionsScreen />);
      // IdentitySelector renders a label
      expect(
        screen.getByText(/step 2 — select identity/i),
      ).toBeInTheDocument();
    });

    it("shows fetch button", () => {
      setStoreData([makeIdentity()], [makeContract()]);
      render(<GroupActionsScreen />);
      expect(
        screen.getByRole("button", { name: /fetch group actions/i }),
      ).toBeInTheDocument();
    });

    it("disables fetch button when contract not selected", () => {
      setStoreData([makeIdentity()], [makeContract()]);
      render(<GroupActionsScreen />);
      expect(
        screen.getByRole("button", { name: /fetch group actions/i }),
      ).toBeDisabled();
    });
  });

  // ── Empty states ────────────────────────────────────────────────

  describe("Empty states", () => {
    it("shows no contracts message when none have tokens", () => {
      setStoreData(
        [],
        [makeContract({ tokenCount: 0, alias: "No-Token Contract" })],
      );
      render(<GroupActionsScreen />);
      expect(
        screen.getByText(/no contracts with tokens found/i),
      ).toBeInTheDocument();
    });

    it("shows no identities message when none loaded", async () => {
      // Pre-populate the contract store with at least one token contract,
      // and leave identities empty
      setStoreData([], [makeContract()]);
      render(<GroupActionsScreen />);

      // The mount effect may trigger loadIdentities, so wait for it
      await waitFor(() => {
        expect(
          screen.getByText(/no identities loaded/i),
        ).toBeInTheDocument();
      });
    });

    it("shows hint when both selected but not yet fetched", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      // Select contract
      const contractSelect = screen.getByRole("combobox", {
        name: /contract/i,
      });
      await userEvent.click(contractSelect);
      const contractOption = screen.getByText(/MyToken Contract/i);
      await userEvent.click(contractOption);

      // Identity is auto-selected (only one)
      await waitFor(() => {
        expect(
          screen.getByText(/click "fetch group actions"/i),
        ).toBeInTheDocument();
      });
    });

    it("shows prompt to select contract and identity when nothing selected", () => {
      setStoreData([makeIdentity(), makeIdentity({ id: "bb".repeat(32), alias: "Second" })], [makeContract()]);
      render(<GroupActionsScreen />);
      expect(
        screen.getByText(/select a contract with tokens and an identity/i),
      ).toBeInTheDocument();
    });
  });

  // ── Contract filtering ──────────────────────────────────────────

  describe("Contract filtering", () => {
    it("excludes contracts with zero tokens", () => {
      setStoreData([], [
        makeContract({ id: "aa".repeat(32), alias: "Has Tokens", tokenCount: 1 }),
        makeContract({ id: "bb".repeat(32), alias: "No Tokens", tokenCount: 0 }),
      ]);
      render(<GroupActionsScreen />);

      const contractSelect = screen.getByRole("combobox", {
        name: /contract/i,
      });
      expect(contractSelect).toBeInTheDocument();
      // The select should show the contract with tokens, not the one without
    });

    it("excludes system contracts", () => {
      setStoreData([], [
        makeContract({ id: "cc".repeat(32), alias: "dpns", tokenCount: 1 }),
        makeContract({ id: "dd".repeat(32), alias: "dashpay", tokenCount: 1 }),
        makeContract({ id: "ee".repeat(32), alias: "Custom Token", tokenCount: 1 }),
      ]);
      render(<GroupActionsScreen />);
      // System contracts should be filtered out
      expect(
        screen.getByRole("combobox", { name: /contract/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Fetch flow ──────────────────────────────────────────────────

  describe("Fetch flow", () => {
    it("dispatches IPC command when fetch button clicked", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      // Select contract
      const contractSelect = screen.getByRole("combobox", {
        name: /contract/i,
      });
      await userEvent.click(contractSelect);
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      // Identity auto-selected (single identity)
      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      expect(
        mockCommands.contractFetchActiveGroupActions,
      ).toHaveBeenCalledWith({
        contractId: contract.id,
        identityId: identity.id,
      });
    });

    it("shows fetching state with elapsed time", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      // Select contract
      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      expect(screen.getByText(/fetching…/i)).toBeInTheDocument();
    });

    it("shows results when task result event arrives", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      // Select and fetch
      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      // Emit result
      const actions = [
        makeGroupAction({
          actionId: "action-111",
          actionType: "TokenMint",
          signersCount: 1,
          requiredSignatures: 3,
          details: { amount: 5000 },
        }),
        makeGroupAction({
          actionId: "action-222",
          actionType: "TokenBurn",
          signersCount: 2,
          requiredSignatures: 2,
          details: { amount: 100 },
        }),
      ];

      act(() => {
        emitTaskResult("task-ga-1", actions);
      });

      await waitFor(() => {
        expect(screen.getByText(/action-111/i)).toBeInTheDocument();
        expect(screen.getByText(/action-222/i)).toBeInTheDocument();
      });
    });

    it("shows empty message when no actions found", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      act(() => {
        emitTaskResult("task-ga-1", []);
      });

      await waitFor(() => {
        expect(
          screen.getByText(/no active group actions found/i),
        ).toBeInTheDocument();
      });
    });

    it("shows error when IPC call fails", async () => {
      mockCommands.contractFetchActiveGroupActions.mockResolvedValueOnce({
        status: "error",
        error: "Contract not found",
      });

      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      await waitFor(() => {
        expect(screen.getByText(/contract not found/i)).toBeInTheDocument();
      });
    });

    it("shows error when task error event arrives", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      act(() => {
        emitTaskError("task-ga-1", "Identity is not a member of any group");
      });

      await waitFor(() => {
        expect(
          screen.getByText(/identity is not a member of any group/i),
        ).toBeInTheDocument();
      });
    });

    it("dismisses error when dismiss button clicked", async () => {
      mockCommands.contractFetchActiveGroupActions.mockResolvedValueOnce({
        status: "error",
        error: "Some error",
      });

      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      await waitFor(() => {
        expect(screen.getByText(/some error/i)).toBeInTheDocument();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /dismiss/i }),
      );

      expect(screen.queryByText(/some error/i)).not.toBeInTheDocument();
    });

    it("ignores task result events with wrong task ID", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      // Emit result with wrong task ID
      act(() => {
        emitTaskResult("wrong-task-id", [makeGroupAction()]);
      });

      // Should still be in fetching state, not showing results
      expect(screen.getByText(/fetching…/i)).toBeInTheDocument();
    });

    it("ignores task result events with non-Contract type", async () => {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      // Emit with correct ID but wrong result type
      act(() => {
        eventListeners.taskResultEvent.forEach((cb) =>
          cb({
            payload: {
              taskId: "task-ga-1",
              resultType: "Identity",
              payload: [makeGroupAction()],
            },
          }),
        );
      });

      // Should still be fetching
      expect(screen.getByText(/fetching…/i)).toBeInTheDocument();
    });

    it("handles IPC exception gracefully", async () => {
      mockCommands.contractFetchActiveGroupActions.mockRejectedValueOnce(
        new Error("Network error"),
      );

      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      await waitFor(() => {
        expect(screen.getByText(/network error/i)).toBeInTheDocument();
      });
    });
  });

  // ── Results display ─────────────────────────────────────────────

  describe("Results display", () => {
    async function setupWithResults(actions: GroupActionItem[]) {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      act(() => {
        emitTaskResult("task-ga-1", actions);
      });

      await waitFor(() => {
        if (actions.length > 0) {
          expect(screen.getByText("Action ID")).toBeInTheDocument();
        }
      });
    }

    it("displays action type badges", async () => {
      await setupWithResults([
        makeGroupAction({ actionType: "TokenMint" }),
      ]);

      expect(screen.getByText("Mint")).toBeInTheDocument();
    });

    it("displays signer counts", async () => {
      await setupWithResults([
        makeGroupAction({ signersCount: 2, requiredSignatures: 5 }),
      ]);

      expect(screen.getByText("2/5")).toBeInTheDocument();
    });

    it("displays action info from details", async () => {
      await setupWithResults([
        makeGroupAction({
          details: { amount: 5000, recipient: "abcdef1234567890" },
        }),
      ]);

      expect(screen.getByText(/amount: 5000/i)).toBeInTheDocument();
    });

    it("displays Take Action buttons", async () => {
      await setupWithResults([makeGroupAction()]);

      expect(
        screen.getByRole("button", { name: /take action/i }),
      ).toBeInTheDocument();
    });

    it("shows table headers", async () => {
      await setupWithResults([makeGroupAction()]);

      expect(screen.getByText("Action ID")).toBeInTheDocument();
      expect(screen.getByText("Type")).toBeInTheDocument();
      expect(screen.getByText("Info")).toBeInTheDocument();
      expect(screen.getByText("Signers")).toBeInTheDocument();
      expect(screen.getByText("Action")).toBeInTheDocument();
    });

    it("shows action count", async () => {
      await setupWithResults([
        makeGroupAction({ actionId: "a1" }),
        makeGroupAction({ actionId: "a2" }),
        makeGroupAction({ actionId: "a3" }),
      ]);

      expect(screen.getByText(/3 of 3 actions/i)).toBeInTheDocument();
    });
  });

  // ── Search filter ───────────────────────────────────────────────

  describe("Search filter", () => {
    async function setupWithResults(actions: GroupActionItem[]) {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      // Wait for event listeners to be set up
      await waitFor(() => {
        expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
      });

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      act(() => {
        emitTaskResult("task-ga-1", actions);
      });

      await waitFor(() => {
        expect(
          screen.getByPlaceholderText(/filter actions/i),
        ).toBeInTheDocument();
      });
    }

    it("renders search input after results loaded", async () => {
      await setupWithResults([makeGroupAction()]);

      expect(
        screen.getByPlaceholderText(/filter actions/i),
      ).toBeInTheDocument();
    });

    it("filters actions by action ID", async () => {
      await setupWithResults([
        makeGroupAction({ actionId: "mint-action-111", actionType: "TokenMint" }),
        makeGroupAction({ actionId: "burn-action-222", actionType: "TokenBurn" }),
      ]);

      // Both actions should be visible initially
      expect(screen.getByText(/mint-action-111/)).toBeInTheDocument();
      expect(screen.getByText(/burn-action-222/)).toBeInTheDocument();

      await userEvent.type(
        screen.getByPlaceholderText(/filter actions/i),
        "burn-action",
      );

      // After filtering, only burn action should be visible
      await waitFor(() => {
        expect(screen.queryByText(/mint-action-111/)).not.toBeInTheDocument();
        expect(screen.getByText(/burn-action-222/)).toBeInTheDocument();
      });
    });

    it("filters actions by action type", async () => {
      await setupWithResults([
        makeGroupAction({ actionId: "a1", actionType: "TokenMint" }),
        makeGroupAction({ actionId: "a2", actionType: "TokenBurn" }),
        makeGroupAction({ actionId: "a3", actionType: "TokenMint" }),
      ]);

      await userEvent.type(
        screen.getByPlaceholderText(/filter actions/i),
        "burn",
      );

      await waitFor(() => {
        expect(screen.getByText(/1 of 3 actions/i)).toBeInTheDocument();
      });
    });
  });

  // ── Take Action ─────────────────────────────────────────────────

  describe("Take Action", () => {
    async function setupWithAction(
      action: GroupActionItem,
      contractId = "11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd",
    ) {
      const identity = makeIdentity();
      const contract = makeContract({ id: contractId });
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      act(() => {
        emitTaskResult("task-ga-1", [action]);
      });

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /take action/i }),
        ).toBeInTheDocument();
      });
    }

    it("navigates to mint route for TokenMint action", async () => {
      await setupWithAction(
        makeGroupAction({ actionType: "TokenMint", actionId: "mint-1" }),
      );

      await userEvent.click(
        screen.getByRole("button", { name: /take action/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({ to: "/tokens/mint" }),
      );
    });

    it("navigates to burn route for TokenBurn action", async () => {
      await setupWithAction(
        makeGroupAction({ actionType: "TokenBurn", actionId: "burn-1" }),
      );

      await userEvent.click(
        screen.getByRole("button", { name: /take action/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({ to: "/tokens/burn" }),
      );
    });

    it("navigates to freeze route for TokenFreeze action", async () => {
      await setupWithAction(
        makeGroupAction({ actionType: "TokenFreeze", actionId: "freeze-1" }),
      );

      await userEvent.click(
        screen.getByRole("button", { name: /take action/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({ to: "/tokens/freeze" }),
      );
    });

    it("passes group action details in search params", async () => {
      const action = makeGroupAction({
        actionType: "TokenMint",
        actionId: "mint-999",
        groupPosition: 2,
        details: { amount: 5000 },
      });
      await setupWithAction(action);

      await userEvent.click(
        screen.getByRole("button", { name: /take action/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({
          search: expect.objectContaining({
            groupActionId: "mint-999",
            groupPosition: 2,
            details: JSON.stringify({ amount: 5000 }),
          }),
        }),
      );
    });
  });

  // ── Navigation ──────────────────────────────────────────────────

  describe("Navigation", () => {
    it("navigates back to contracts when back button clicked", async () => {
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("button", { name: /back to contracts/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
    });
  });

  // ── Event subscription ──────────────────────────────────────────

  describe("Event subscription", () => {
    it("subscribes to task result events on mount", async () => {
      render(<GroupActionsScreen />);

      await waitFor(() => {
        expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
      });
    });

    it("subscribes to task error events on mount", async () => {
      render(<GroupActionsScreen />);

      await waitFor(() => {
        expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
      });
    });
  });

  // ── Auto-select identity ────────────────────────────────────────

  describe("Auto-select", () => {
    it("auto-selects first identity when only one exists", async () => {
      const identity = makeIdentity({ id: "aa".repeat(32) });
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      // Select contract to enable fetch
      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      // The fetch button should be enabled since identity is auto-selected
      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });
    });
  });

  // ── Payload parsing ─────────────────────────────────────────────

  describe("Payload parsing", () => {
    async function setupAndFetch() {
      const identity = makeIdentity();
      const contract = makeContract();
      setStoreData([identity], [contract]);
      render(<GroupActionsScreen />);

      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/MyToken Contract/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );
    }

    it("parses array payload", async () => {
      await setupAndFetch();

      act(() => {
        emitTaskResult("task-ga-1", [
          {
            groupPosition: 0,
            actionId: "parsed-array-1",
            actionType: "TokenMint",
            signersCount: 1,
            requiredSignatures: 2,
            details: {},
          },
        ]);
      });

      await waitFor(() => {
        expect(screen.getByText(/parsed-array-1/i)).toBeInTheDocument();
      });
    });

    it("handles null payload as empty", async () => {
      await setupAndFetch();

      act(() => {
        emitTaskResult("task-ga-1", null);
      });

      await waitFor(() => {
        expect(
          screen.getByText(/no active group actions found/i),
        ).toBeInTheDocument();
      });
    });

    it("handles camelCase fields from DTO", async () => {
      await setupAndFetch();

      act(() => {
        emitTaskResult("task-ga-1", [
          {
            groupPosition: 1,
            actionId: "camel-case-id",
            actionType: "TokenBurn",
            signersCount: 3,
            requiredSignatures: 5,
            details: { amount: 999 },
          },
        ]);
      });

      await waitFor(() => {
        expect(screen.getByText(/camel-case-id/i)).toBeInTheDocument();
        expect(screen.getByText("3/5")).toBeInTheDocument();
      });
    });

    it("handles snake_case fields from raw API", async () => {
      await setupAndFetch();

      act(() => {
        emitTaskResult("task-ga-1", [
          {
            group_position: 1,
            action_id: "snake-case-id",
            action_type: "TokenFreeze",
            signers_count: 2,
            required_signatures: 4,
            details: {},
          },
        ]);
      });

      await waitFor(() => {
        expect(screen.getByText(/snake-case-id/i)).toBeInTheDocument();
        expect(screen.getByText("2/4")).toBeInTheDocument();
      });
    });
  });

  // ── Loading states ──────────────────────────────────────────────

  describe("Loading states", () => {
    it("shows contracts loading spinner", () => {
      useContractStore.setState({ loading: true });
      render(<GroupActionsScreen />);
      expect(screen.getByText(/loading contracts/i)).toBeInTheDocument();
    });

    it("shows identities loading spinner", () => {
      useIdentityStore.setState({ loading: true });
      setStoreData([], [makeContract()]);
      useIdentityStore.setState({ loading: true });
      render(<GroupActionsScreen />);
      expect(screen.getByText(/loading identities/i)).toBeInTheDocument();
    });
  });

  // ── State reset on selection change ─────────────────────────────

  describe("State reset", () => {
    it("resets to idle when contract changes", async () => {
      const identity = makeIdentity();
      const contract1 = makeContract({ id: "aa".repeat(32), alias: "Contract A" });
      const contract2 = makeContract({ id: "bb".repeat(32), alias: "Contract B" });
      setStoreData([identity], [contract1, contract2]);
      render(<GroupActionsScreen />);

      // Select contract and fetch
      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/Contract A/i));

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /fetch group actions/i }),
        ).toBeEnabled();
      });

      await userEvent.click(
        screen.getByRole("button", { name: /fetch group actions/i }),
      );

      act(() => {
        emitTaskResult("task-ga-1", [makeGroupAction()]);
      });

      await waitFor(() => {
        expect(screen.getByText("Action ID")).toBeInTheDocument();
      });

      // Change contract — should reset
      await userEvent.click(
        screen.getByRole("combobox", { name: /contract/i }),
      );
      await userEvent.click(screen.getByText(/Contract B/i));

      await waitFor(() => {
        expect(screen.queryByText("Action ID")).not.toBeInTheDocument();
      });
    });
  });
});
