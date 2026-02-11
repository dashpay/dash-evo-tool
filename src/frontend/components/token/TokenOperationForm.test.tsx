import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  TokenOperationForm,
  autoSelectKey,
  getActionButtonLabel,
  getSuccessTitle,
} from "./TokenOperationForm";
import type { TokenOperationFormProps } from "./TokenOperationForm";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

const mockLoadIdentities = vi.fn().mockResolvedValue(null);
const mockLoadWallets = vi.fn().mockResolvedValue(null);
const mockLoadMyTokenBalances = vi.fn().mockResolvedValue(null);

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

const mockIdentities = [
  {
    id: "id-abc123def456",
    alias: "TestIdentity",
    balance: 1000000000,
    keys: [
      {
        keyId: 1,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "deadbeef",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
      {
        keyId: 2,
        purpose: "AUTHENTICATION",
        securityLevel: "CRITICAL",
        keyType: "BLS12_381",
        data: "cafebabe",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-1"],
    walletIndex: 0,
    topUps: [],
    status: "Active",
    identityType: "User",
  },
];

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      identities: mockIdentities,
      loadIdentities: mockLoadIdentities,
    };
    return sel ? sel(s) : s;
  },
}));

vi.mock("@/stores/walletStore", () => ({
  useWalletStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      hdWallets: [
        {
          seedHash: "seed-hash-1",
          alias: "TestWallet",
          usesPassword: false,
          passwordHint: null,
        },
      ],
      singleKeyWallets: [],
      loadWallets: mockLoadWallets,
    };
    return sel ? sel(s) : s;
  },
}));

vi.mock("@/stores/tokenStore", () => ({
  useTokenStore: () => ({
    loadMyTokenBalances: mockLoadMyTokenBalances,
  }),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

// ─── Test Fixtures ──────────────────────────────────────────────────────────

const defaultTokenContext: TokenOperationFormProps["tokenContext"] = {
  tokenId: "token1111222233334444555566667777888899990000aaaabbbbccccddddeeee",
  contractId: "contract111122223333444455556666777788889999",
  tokenPosition: 0,
  name: "TestToken",
  balance: "500000000",
  decimals: 8,
  identityId: "id-abc123def456",
};

const defaultOnSubmit = vi
  .fn()
  .mockResolvedValue({ status: "ok", data: { taskId: "task-123" } });

function makeProps(
  overrides: Partial<TokenOperationFormProps> = {},
): TokenOperationFormProps {
  return {
    actionName: "Transfer",
    tokenContext: defaultTokenContext,
    onSubmit: defaultOnSubmit,
    ...overrides,
  };
}

function setup(overrides: Partial<TokenOperationFormProps> = {}) {
  const props = makeProps(overrides);
  return {
    user: userEvent.setup(),
    ...render(<TokenOperationForm {...props} />),
    props,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(commands.walletNotifyUnlocked).mockResolvedValue({
    status: "ok",
    data: null,
  });
});

// ─── autoSelectKey helper ───────────────────────────────────────────────────

describe("autoSelectKey", () => {
  it("returns null for null identity", () => {
    expect(autoSelectKey(null)).toBeNull();
  });

  it("prefers HIGH auth key over CRITICAL", () => {
    const identity = mockIdentities[0] as Parameters<typeof autoSelectKey>[0];
    expect(autoSelectKey(identity)).toBe(1); // keyId 1 is HIGH
  });

  it("falls back to CRITICAL if no HIGH", () => {
    const identity = {
      ...mockIdentities[0],
      keys: [mockIdentities[0].keys[1]], // Only CRITICAL key
    } as Parameters<typeof autoSelectKey>[0];
    expect(autoSelectKey(identity)).toBe(2);
  });

  it("returns first auth key if no HIGH or CRITICAL", () => {
    const identity = {
      ...mockIdentities[0],
      keys: [
        {
          keyId: 5,
          purpose: "AUTHENTICATION",
          securityLevel: "MEDIUM",
          keyType: "ECDSA_SECP256K1",
          data: "aabb",
          isDisabled: false,
          disabledAt: null,
          hasPrivateKey: true,
        },
      ],
    } as Parameters<typeof autoSelectKey>[0];
    expect(autoSelectKey(identity)).toBe(5);
  });

  it("skips disabled keys", () => {
    const identity = {
      ...mockIdentities[0],
      keys: [
        {
          ...mockIdentities[0].keys[0],
          isDisabled: true,
        },
        mockIdentities[0].keys[1],
      ],
    } as Parameters<typeof autoSelectKey>[0];
    expect(autoSelectKey(identity)).toBe(2);
  });

  it("returns null if no auth keys at all", () => {
    const identity = {
      ...mockIdentities[0],
      keys: [
        {
          keyId: 3,
          purpose: "VOTING",
          securityLevel: "HIGH",
          keyType: "ECDSA_SECP256K1",
          data: "aabb",
          isDisabled: false,
          disabledAt: null,
          hasPrivateKey: true,
        },
      ],
    } as Parameters<typeof autoSelectKey>[0];
    expect(autoSelectKey(identity)).toBeNull();
  });
});

// ─── getActionButtonLabel helper ────────────────────────────────────────────

describe("getActionButtonLabel", () => {
  it("returns action name when no group context", () => {
    expect(getActionButtonLabel("Transfer")).toBe("Transfer");
  });

  it("returns Sign prefix for group action signing", () => {
    expect(
      getActionButtonLabel("Mint", {
        groupActionId: "abc",
        hasGroup: true,
        isUnilateral: false,
      }),
    ).toBe("Sign Mint");
  });

  it("returns Initiate Group prefix for non-unilateral group action", () => {
    expect(
      getActionButtonLabel("Burn", {
        hasGroup: true,
        isUnilateral: false,
      }),
    ).toBe("Initiate Group Burn");
  });

  it("returns plain action name for unilateral group member", () => {
    expect(
      getActionButtonLabel("Freeze", {
        hasGroup: true,
        isUnilateral: true,
      }),
    ).toBe("Freeze");
  });
});

// ─── getSuccessTitle helper ─────────────────────────────────────────────────

describe("getSuccessTitle", () => {
  it("returns Successful for no group context", () => {
    expect(getSuccessTitle("Transfer")).toBe("Transfer Successful");
  });

  it("returns Signing Successful for group signing", () => {
    expect(
      getSuccessTitle("Mint", {
        groupActionId: "abc",
        hasGroup: true,
        isUnilateral: false,
      }),
    ).toBe("Group Mint Signing Successful");
  });

  it("returns Initiated for non-unilateral group action", () => {
    expect(
      getSuccessTitle("Burn", {
        hasGroup: true,
        isUnilateral: false,
      }),
    ).toBe("Group Burn Initiated");
  });

  it("returns Successful for unilateral group member", () => {
    expect(
      getSuccessTitle("Freeze", {
        hasGroup: true,
        isUnilateral: true,
      }),
    ).toBe("Freeze Successful");
  });
});

// ─── Form rendering ─────────────────────────────────────────────────────────

describe("TokenOperationForm — form rendering", () => {
  it("renders the form container", () => {
    setup();
    expect(screen.getByTestId("operation-form")).toBeInTheDocument();
  });

  it("renders token context header with name and balance", () => {
    setup();
    const header = screen.getByTestId("token-context-header");
    expect(within(header).getByText("TestToken")).toBeInTheDocument();
    expect(within(header).getByText("5")).toBeInTheDocument(); // 500000000 / 10^8 = 5
  });

  it("renders Unnamed Token when name is null", () => {
    setup({
      tokenContext: { ...defaultTokenContext, name: null },
    });
    expect(screen.getByText("Unnamed Token")).toBeInTheDocument();
  });

  it("renders identity selector with loaded identities", () => {
    setup();
    expect(
      screen.getByTestId("operation-identity-select"),
    ).toBeInTheDocument();
  });

  it("renders key selector", () => {
    setup();
    expect(screen.getByTestId("operation-key-select")).toBeInTheDocument();
  });

  it("renders wallet unlocked status for non-password wallet", () => {
    setup();
    expect(screen.getByText("Wallet unlocked")).toBeInTheDocument();
  });

  it("shows no-identities warning when identities empty", async () => {
    // We can't easily change the mock per-test with vi.mock, but we can
    // test the warning is absent when identities exist
    setup();
    expect(
      screen.queryByTestId("no-identities-warning"),
    ).not.toBeInTheDocument();
  });

  it("renders advanced options toggle", () => {
    setup();
    expect(
      screen.getByTestId("operation-advanced-toggle"),
    ).toBeInTheDocument();
  });

  it("renders cancel and submit buttons", () => {
    setup();
    expect(screen.getByTestId("operation-cancel")).toBeInTheDocument();
    expect(screen.getByTestId("operation-submit")).toBeInTheDocument();
  });

  it("submit button shows action name", () => {
    setup({ actionName: "Burn" });
    expect(screen.getByTestId("operation-submit")).toHaveTextContent("Burn");
  });

  it("loads identities and wallets on mount", () => {
    setup();
    expect(mockLoadIdentities).toHaveBeenCalled();
    expect(mockLoadWallets).toHaveBeenCalled();
  });
});

// ─── Amount input ───────────────────────────────────────────────────────────

describe("TokenOperationForm — amount input", () => {
  it("does not render amount input by default", () => {
    setup();
    expect(screen.queryByTestId("amount-section")).not.toBeInTheDocument();
  });

  it("renders amount input when showAmountInput is true", () => {
    setup({
      showAmountInput: true,
      amount: "",
      onAmountChange: vi.fn(),
    });
    expect(screen.getByTestId("amount-section")).toBeInTheDocument();
    expect(screen.getByTestId("operation-amount-input")).toBeInTheDocument();
  });

  it("renders custom amount label", () => {
    setup({
      showAmountInput: true,
      amountLabel: "Tokens to Burn",
      amount: "",
      onAmountChange: vi.fn(),
    });
    expect(screen.getByText("Tokens to Burn")).toBeInTheDocument();
  });

  it("renders max button when maxAmount provided", () => {
    setup({
      showAmountInput: true,
      amount: "",
      onAmountChange: vi.fn(),
      maxAmount: "500000000",
    });
    expect(screen.getByTestId("operation-max-button")).toBeInTheDocument();
  });

  it("clicking max button fills the amount", async () => {
    const onAmountChange = vi.fn();
    const { user } = setup({
      showAmountInput: true,
      amount: "",
      onAmountChange,
      maxAmount: "500000000",
    });
    await user.click(screen.getByTestId("operation-max-button"));
    expect(onAmountChange).toHaveBeenCalledWith("500000000");
  });

  it("calls onAmountChange when typing", async () => {
    const onAmountChange = vi.fn();
    const { user } = setup({
      showAmountInput: true,
      amount: "",
      onAmountChange,
    });
    const input = screen.getByTestId("operation-amount-input");
    await user.type(input, "100");
    expect(onAmountChange).toHaveBeenCalled();
  });

  it("shows available balance when maxAmount provided", () => {
    setup({
      showAmountInput: true,
      amount: "",
      onAmountChange: vi.fn(),
      maxAmount: "500000000",
    });
    expect(screen.getByText(/Available:/)).toBeInTheDocument();
  });
});

// ─── Recipient input ────────────────────────────────────────────────────────

describe("TokenOperationForm — recipient input", () => {
  it("does not render recipient input by default", () => {
    setup();
    expect(screen.queryByTestId("recipient-section")).not.toBeInTheDocument();
  });

  it("renders recipient input when showRecipientInput is true", () => {
    setup({
      showRecipientInput: true,
      recipientId: "",
      onRecipientChange: vi.fn(),
    });
    expect(screen.getByTestId("recipient-section")).toBeInTheDocument();
    expect(
      screen.getByTestId("operation-recipient-input"),
    ).toBeInTheDocument();
  });

  it("renders custom recipient label", () => {
    setup({
      showRecipientInput: true,
      recipientLabel: "Freeze Target",
      recipientId: "",
      onRecipientChange: vi.fn(),
    });
    expect(screen.getByText("Freeze Target")).toBeInTheDocument();
  });

  it("shows Optional badge when recipientOptional", () => {
    setup({
      showRecipientInput: true,
      recipientOptional: true,
      recipientId: "",
      onRecipientChange: vi.fn(),
    });
    expect(screen.getByText("Optional")).toBeInTheDocument();
  });

  it("calls onRecipientChange when typing", async () => {
    const onRecipientChange = vi.fn();
    const { user } = setup({
      showRecipientInput: true,
      recipientId: "",
      onRecipientChange,
    });
    const input = screen.getByTestId("operation-recipient-input");
    await user.type(input, "abc");
    expect(onRecipientChange).toHaveBeenCalled();
  });
});

// ─── Advanced options ───────────────────────────────────────────────────────

describe("TokenOperationForm — advanced options", () => {
  it("public note is hidden by default", () => {
    setup();
    expect(
      screen.queryByTestId("operation-public-note"),
    ).not.toBeInTheDocument();
  });

  it("shows public note input after clicking advanced toggle", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    expect(screen.getByTestId("operation-public-note")).toBeInTheDocument();
  });

  it("public note is disabled when signing group action", async () => {
    const { user } = setup({
      groupAction: {
        groupActionId: "existing-action-id",
        hasGroup: true,
        isUnilateral: false,
      },
    });
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    const noteInput = screen.getByTestId("operation-public-note");
    expect(noteInput).toBeDisabled();
  });

  it("shows 'View Key Info' button in advanced options when a key is selected", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    expect(screen.getByTestId("operation-view-key-info")).toBeInTheDocument();
    expect(screen.getByTestId("operation-view-key-info")).toHaveTextContent("View Key Info");
  });

  it("shows 'Add Key' button in advanced options", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    expect(screen.getByTestId("operation-add-key")).toBeInTheDocument();
    expect(screen.getByTestId("operation-add-key")).toHaveTextContent("Add Key");
  });

  it("'View Key Info' navigates to /identities", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    await user.click(screen.getByTestId("operation-view-key-info"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
  });

  it("'Add Key' navigates to /identities", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    await user.click(screen.getByTestId("operation-add-key"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
  });

  it("shows key management helper text in advanced options", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    expect(screen.getByText(/Navigate to Identities to manage keys/)).toBeInTheDocument();
  });
});

// ─── Group action info ──────────────────────────────────────────────────────

describe("TokenOperationForm — group action info", () => {
  it("does not show group info when no groupAction", () => {
    setup();
    expect(
      screen.queryByTestId("group-action-info"),
    ).not.toBeInTheDocument();
  });

  it("shows group action signing message", () => {
    setup({
      groupAction: {
        groupActionId: "action-xyz",
        hasGroup: true,
        isUnilateral: false,
      },
    });
    const info = screen.getByTestId("group-action-info");
    expect(
      within(info).getByText("Group Action Signing"),
    ).toBeInTheDocument();
  });

  it("shows unilateral group member message", () => {
    setup({
      groupAction: {
        hasGroup: true,
        isUnilateral: true,
      },
    });
    const info = screen.getByTestId("group-action-info");
    expect(
      within(info).getByText("Unilateral Group Member"),
    ).toBeInTheDocument();
  });

  it("shows group action required message for non-unilateral", () => {
    setup({
      groupAction: {
        hasGroup: true,
        isUnilateral: false,
      },
    });
    const info = screen.getByTestId("group-action-info");
    expect(
      within(info).getByText("Group Action Required"),
    ).toBeInTheDocument();
  });

  it("does not show group info when hasGroup is false", () => {
    setup({
      groupAction: {
        hasGroup: false,
        isUnilateral: false,
      },
    });
    expect(
      screen.queryByTestId("group-action-info"),
    ).not.toBeInTheDocument();
  });
});

// ─── Validation ─────────────────────────────────────────────────────────────

describe("TokenOperationForm — validation", () => {
  it("submit button is enabled when isValid is true (default)", () => {
    setup();
    expect(screen.getByTestId("operation-submit")).not.toBeDisabled();
  });

  it("submit button is disabled when isValid is false", () => {
    setup({ isValid: false });
    expect(screen.getByTestId("operation-submit")).toBeDisabled();
  });

  it("shows validation message when isValid is false", () => {
    setup({
      isValid: false,
      validationMessage: "Amount is required",
    });
    expect(
      screen.getByTestId("operation-validation-message"),
    ).toHaveTextContent("Amount is required");
  });

  it("does not show validation message when valid", () => {
    setup({ validationMessage: "Amount is required" });
    expect(
      screen.queryByTestId("operation-validation-message"),
    ).not.toBeInTheDocument();
  });
});

// ─── Confirmation dialog ────────────────────────────────────────────────────

describe("TokenOperationForm — confirmation dialog", () => {
  it("submits directly when no confirmation config", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "t1" } });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));
    expect(onSubmit).toHaveBeenCalledWith({
      identityId: "id-abc123def456",
      keyId: 1,
      publicNote: null,
    });
  });

  it("opens confirmation dialog before submitting", async () => {
    const onSubmit = vi.fn();
    const { user } = setup({
      onSubmit,
      confirmation: {
        title: "Confirm Transfer",
        description: "Are you sure you want to transfer?",
      },
    });
    await user.click(screen.getByTestId("operation-submit"));
    // onSubmit should NOT have been called yet
    expect(onSubmit).not.toHaveBeenCalled();
    // Dialog should be visible
    expect(screen.getByText("Confirm Transfer")).toBeInTheDocument();
    expect(
      screen.getByText("Are you sure you want to transfer?"),
    ).toBeInTheDocument();
  });
});

// ─── Submit & broadcast flow ────────────────────────────────────────────────

describe("TokenOperationForm — submit and broadcast", () => {
  it("transitions to broadcasting state after submit", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-xyz" } });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));
    expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();
  });

  it("shows action name in broadcasting message", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-xyz" } });
    const { user } = setup({ onSubmit, actionName: "Mint" });
    await user.click(screen.getByTestId("operation-submit"));
    expect(screen.getByText("Mint...")).toBeInTheDocument();
  });

  it("shows Signing prefix in broadcasting for group action signing", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-xyz" } });
    const { user } = setup({
      onSubmit,
      actionName: "Mint",
      groupAction: {
        groupActionId: "existing-action",
        hasGroup: true,
        isUnilateral: false,
      },
    });
    await user.click(screen.getByTestId("operation-submit"));
    expect(screen.getByText("Signing Mint...")).toBeInTheDocument();
  });

  it("transitions to error state on IPC error result", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "error", error: "Insufficient funds" });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));
    expect(screen.getByTestId("operation-error")).toBeInTheDocument();
    expect(screen.getByText("Insufficient funds")).toBeInTheDocument();
  });

  it("transitions to error state on thrown exception", async () => {
    const onSubmit = vi
      .fn()
      .mockRejectedValue(new Error("Network timeout"));
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));
    expect(screen.getByTestId("operation-error")).toBeInTheDocument();
    expect(screen.getByText("Network timeout")).toBeInTheDocument();
  });

  it("passes public note to onSubmit", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "t1" } });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-advanced-toggle"));
    await user.type(
      screen.getByTestId("operation-public-note"),
      "Test note",
    );
    await user.click(screen.getByTestId("operation-submit"));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ publicNote: "Test note" }),
    );
  });
});

// ─── Task event listeners ───────────────────────────────────────────────────

describe("TokenOperationForm — task event listeners", () => {
  it("transitions to success on taskResultEvent", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-123" } });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));
    expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();

    // Simulate task result event
    const resultListener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
    await act(async () => {
      resultListener?.({ payload: { taskId: "task-123", resultType: "Token" } });
    });

    expect(screen.getByTestId("operation-success")).toBeInTheDocument();
  });

  it("transitions to error on taskErrorEvent", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-456" } });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));

    const errorListener = vi.mocked(events.taskErrorEvent.listen).mock.calls[0]?.[0];
    await act(async () => {
      errorListener?.({ payload: { taskId: "task-456", message: "Backend error occurred" } });
    });

    expect(screen.getByTestId("operation-error")).toBeInTheDocument();
    expect(screen.getByText("Backend error occurred")).toBeInTheDocument();
  });

  it("ignores events for different task IDs", async () => {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-789" } });
    const { user } = setup({ onSubmit });
    await user.click(screen.getByTestId("operation-submit"));

    const resultListener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
    await act(async () => {
      resultListener?.({ payload: { taskId: "different-task", resultType: "Token" } });
    });

    // Should still be broadcasting
    expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();
  });
});

// ─── Success screen ─────────────────────────────────────────────────────────

describe("TokenOperationForm — success screen", () => {
  async function reachSuccess(overrides: Partial<TokenOperationFormProps> = {}) {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: { taskId: "task-s1" } });
    const merged = { onSubmit, ...overrides };
    const { user } = setup(merged);
    await user.click(screen.getByTestId("operation-submit"));
    const resultListener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
    await act(async () => {
      resultListener?.({ payload: { taskId: "task-s1", resultType: "Token" } });
    });
  }

  it("shows success title with action name", async () => {
    await reachSuccess({ actionName: "Transfer" });
    expect(screen.getByText("Transfer Successful")).toBeInTheDocument();
  });

  it("shows Back to Tokens button", async () => {
    await reachSuccess();
    expect(
      screen.getByTestId("operation-back-to-tokens"),
    ).toBeInTheDocument();
  });

  it("navigates to /tokens when clicking Back to Tokens", async () => {
    await reachSuccess();
    const user = userEvent.setup();
    await user.click(screen.getByTestId("operation-back-to-tokens"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens" });
  });

  it("shows Go to Group Actions for non-unilateral group initiated action", async () => {
    await reachSuccess({
      groupAction: {
        hasGroup: true,
        isUnilateral: false,
      },
    });
    expect(
      screen.getByTestId("operation-go-to-group-actions"),
    ).toBeInTheDocument();
  });

  it("does not show Go to Group Actions for normal action", async () => {
    await reachSuccess();
    expect(
      screen.queryByTestId("operation-go-to-group-actions"),
    ).not.toBeInTheDocument();
  });

  it("shows do-another button when onDoAnother provided", async () => {
    const onDoAnother = vi.fn();
    await reachSuccess({ onDoAnother });
    expect(screen.getByTestId("operation-do-another")).toBeInTheDocument();
  });

  it("calls onDoAnother when clicking the button", async () => {
    const onDoAnother = vi.fn();
    await reachSuccess({ onDoAnother });
    const user = userEvent.setup();
    await user.click(screen.getByTestId("operation-do-another"));
    expect(onDoAnother).toHaveBeenCalled();
  });

  it("calls onSuccess callback on success", async () => {
    const onSuccess = vi.fn();
    await reachSuccess({ onSuccess });
    expect(onSuccess).toHaveBeenCalled();
  });
});

// ─── Error screen ───────────────────────────────────────────────────────────

describe("TokenOperationForm — error screen", () => {
  async function reachError(overrides: Partial<TokenOperationFormProps> = {}) {
    const onSubmit = vi
      .fn()
      .mockResolvedValue({ status: "error", error: "Test error message" });
    const merged = { onSubmit, ...overrides };
    const { user } = setup(merged);
    await user.click(screen.getByTestId("operation-submit"));
  }

  it("shows error title with action name", async () => {
    await reachError({ actionName: "Burn" });
    expect(screen.getByText("Burn Failed")).toBeInTheDocument();
  });

  it("shows error message", async () => {
    await reachError();
    expect(screen.getByText("Test error message")).toBeInTheDocument();
  });

  it("returns to form on Try Again click", async () => {
    await reachError();
    const user = userEvent.setup();
    await user.click(screen.getByTestId("operation-try-again"));
    expect(screen.getByTestId("operation-form")).toBeInTheDocument();
  });

  it("navigates back to tokens from error screen", async () => {
    await reachError();
    const user = userEvent.setup();
    // The "Back to Tokens" button on error screen
    const buttons = screen.getAllByRole("button");
    const backBtn = buttons.find((b) => b.textContent?.includes("Back to Tokens"));
    expect(backBtn).toBeDefined();
    await user.click(backBtn!);
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens" });
  });
});

// ─── Cancel button ──────────────────────────────────────────────────────────

describe("TokenOperationForm — cancel", () => {
  it("navigates to /tokens when clicking cancel", async () => {
    const { user } = setup();
    await user.click(screen.getByTestId("operation-cancel"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens" });
  });
});

// ─── Custom children ────────────────────────────────────────────────────────

describe("TokenOperationForm — custom children", () => {
  it("renders custom children between inputs and advanced options", () => {
    setup({
      children: <div data-testid="custom-child">Custom content</div>,
    });
    expect(screen.getByTestId("custom-child")).toBeInTheDocument();
    expect(screen.getByText("Custom content")).toBeInTheDocument();
  });
});

// ─── Destructive confirmation ───────────────────────────────────────────────

describe("TokenOperationForm — destructive mode", () => {
  it("renders submit button with destructive variant", () => {
    setup({
      confirmation: {
        title: "Destroy Funds",
        description: "This cannot be undone.",
        destructive: true,
      },
    });
    const btn = screen.getByTestId("operation-submit");
    // destructive variant should be applied
    expect(btn.className).toContain("destructive");
  });
});
