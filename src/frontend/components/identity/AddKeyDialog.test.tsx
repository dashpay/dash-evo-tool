import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { AddKeyDialog, type AddKeyStatus } from "./AddKeyDialog";
import type { QualifiedIdentityDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aabbccdd11223344556677889900aabb",
    identityType: "user",
    alias: "Alice",
    balance: 1_000_000_000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

const defaultProps = {
  identity: makeIdentity(),
  status: { type: "idle" } as AddKeyStatus,
  onSubmit: vi.fn(),
  onDismissError: vi.fn(),
  onBackToKeys: vi.fn(),
  onAddAnother: vi.fn(),
  onBack: vi.fn(),
};

function setup(
  props: Partial<Parameters<typeof AddKeyDialog>[0]> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <AddKeyDialog {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("AddKeyDialog — header", () => {
  it("renders title and identity name", () => {
    setup();
    expect(screen.getByText("Add New Key")).toBeInTheDocument();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("shows truncated ID when no alias", () => {
    setup({ identity: makeIdentity({ alias: null }) });
    expect(screen.getByText(/aabbccdd…00aabb/)).toBeInTheDocument();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /go back/i }));
    expect(props.onBack).toHaveBeenCalledOnce();
  });
});

// ─── Form Fields ────────────────────────────────────────────────────

describe("AddKeyDialog — form fields", () => {
  it("renders Purpose selector", () => {
    setup();
    expect(screen.getByLabelText("Key purpose")).toBeInTheDocument();
  });

  it("renders Security Level selector", () => {
    setup();
    expect(screen.getByLabelText("Security level")).toBeInTheDocument();
  });

  it("renders Key Type selector", () => {
    setup();
    expect(screen.getByLabelText("Key type")).toBeInTheDocument();
  });

  it("renders Private Key input", () => {
    setup();
    expect(screen.getByLabelText("Private key hex")).toBeInTheDocument();
  });

  it("renders Generate Random button", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /generate random/i }),
    ).toBeInTheDocument();
  });

  it("renders Contract Bounds checkbox", () => {
    setup();
    expect(
      screen.getByLabelText("Enable Contract Bounds"),
    ).toBeInTheDocument();
  });

  it("renders Add Key submit button", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /add key/i }),
    ).toBeInTheDocument();
  });
});

// ─── Generate Random Key ────────────────────────────────────────────

describe("AddKeyDialog — generate random key", () => {
  it("fills private key input when Generate Random clicked", async () => {
    const { user } = setup();
    const input = screen.getByLabelText("Private key hex");
    expect(input).toHaveValue("");
    await user.click(
      screen.getByRole("button", { name: /generate random/i }),
    );
    expect(input).not.toHaveValue("");
    // Should be 64 hex chars (32 bytes)
    expect((input as HTMLInputElement).value).toMatch(/^[0-9a-f]{64}$/);
  });
});

// ─── Contract Bounds ────────────────────────────────────────────────

describe("AddKeyDialog — contract bounds", () => {
  it("hides contract fields when checkbox is unchecked", () => {
    setup();
    expect(
      screen.queryByLabelText("Contract ID"),
    ).not.toBeInTheDocument();
  });

  it("shows contract fields when checkbox is checked", async () => {
    const { user } = setup();
    await user.click(screen.getByLabelText("Enable Contract Bounds"));
    expect(screen.getByLabelText("Contract ID")).toBeInTheDocument();
    expect(screen.getByLabelText("Document type name")).toBeInTheDocument();
  });

  it("auto-selects Encryption purpose when bounds enabled", async () => {
    const { user } = setup();
    await user.click(screen.getByLabelText("Enable Contract Bounds"));
    // The purpose trigger should display "Encryption"
    const purposeTrigger = screen.getByLabelText("Key purpose");
    expect(purposeTrigger).toHaveTextContent("Encryption");
  });
});

// ─── Validation ─────────────────────────────────────────────────────

describe("AddKeyDialog — validation", () => {
  it("shows error when submitting without private key", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(screen.getByRole("alert")).toHaveTextContent(/private key is required/i);
  });

  it("shows error for invalid hex in private key", async () => {
    const { user } = setup();
    await user.type(screen.getByLabelText("Private key hex"), "ZZZZ");
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(screen.getByRole("alert")).toHaveTextContent(/hex string/i);
  });

  it("shows error for wrong length private key", async () => {
    const { user } = setup();
    await user.type(screen.getByLabelText("Private key hex"), "aabb");
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(screen.getByRole("alert")).toHaveTextContent(/32 bytes/i);
  });

  it("shows error when contract bounds enabled but no contract ID", async () => {
    const { user } = setup();
    await user.click(screen.getByLabelText("Enable Contract Bounds"));
    // Generate valid key
    await user.click(
      screen.getByRole("button", { name: /generate random/i }),
    );
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(screen.getByRole("alert")).toHaveTextContent(/contract id is required/i);
  });
});

// ─── Submit ─────────────────────────────────────────────────────────

describe("AddKeyDialog — submit", () => {
  it("calls onSubmit with form values", async () => {
    const { user, props } = setup();
    // Generate a random key
    await user.click(
      screen.getByRole("button", { name: /generate random/i }),
    );
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(props.onSubmit).toHaveBeenCalledOnce();
    const call = props.onSubmit!.mock.calls[0][0];
    expect(call.purpose).toBe("AUTHENTICATION");
    expect(call.securityLevel).toBe("CRITICAL");
    expect(call.keyType).toBe("ECDSA_SECP256K1");
    expect(call.privateKeyHex).toMatch(/^[0-9a-f]{64}$/);
    expect(call.contractBounds).toBeUndefined();
  });

  it("includes contract bounds when enabled", async () => {
    const { user, props } = setup();
    await user.click(screen.getByLabelText("Enable Contract Bounds"));
    await user.type(
      screen.getByLabelText("Contract ID"),
      "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    );
    await user.type(
      screen.getByLabelText("Document type name"),
      "profile",
    );
    await user.click(
      screen.getByRole("button", { name: /generate random/i }),
    );
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(props.onSubmit).toHaveBeenCalledOnce();
    const call = props.onSubmit!.mock.calls[0][0];
    expect(call.contractBounds).toEqual({
      contractId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
      documentTypeName: "profile",
    });
  });
});

// ─── Error State ────────────────────────────────────────────────────

describe("AddKeyDialog — error state", () => {
  it("renders error from status", () => {
    setup({
      status: { type: "error", message: "Backend error: insufficient balance" },
    });
    expect(
      screen.getByText("Backend error: insufficient balance"),
    ).toBeInTheDocument();
  });

  it("calls onDismissError when dismiss clicked", async () => {
    const { user, props } = setup({
      status: { type: "error", message: "Error" },
    });
    await user.click(
      screen.getByRole("button", { name: /dismiss error/i }),
    );
    expect(props.onDismissError).toHaveBeenCalledOnce();
  });
});

// ─── Submitting State ───────────────────────────────────────────────

describe("AddKeyDialog — submitting state", () => {
  it("shows loading indicator", () => {
    setup({
      status: { type: "submitting", startedAt: Date.now() },
    });
    expect(screen.getByText("Adding Key…")).toBeInTheDocument();
  });

  it("shows progress message", () => {
    setup({
      status: { type: "submitting", startedAt: Date.now() - 5000 },
    });
    expect(screen.getByText(/this may take a moment/i)).toBeInTheDocument();
  });

  it("has status role", () => {
    setup({
      status: { type: "submitting", startedAt: Date.now() },
    });
    expect(
      screen.getByRole("status", { name: /adding key/i }),
    ).toBeInTheDocument();
  });
});

// ─── Success State ──────────────────────────────────────────────────

describe("AddKeyDialog — success state", () => {
  it("shows success message", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByText("Key Added Successfully"),
    ).toBeInTheDocument();
  });

  it("renders Back to Keys button", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByRole("button", { name: /back to keys/i }),
    ).toBeInTheDocument();
  });

  it("renders Add Another Key button", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByRole("button", { name: /add another key/i }),
    ).toBeInTheDocument();
  });

  it("calls onBackToKeys when clicked", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /back to keys/i }),
    );
    expect(props.onBackToKeys).toHaveBeenCalledOnce();
  });

  it("calls onAddAnother when clicked", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /add another key/i }),
    );
    expect(props.onAddAnother).toHaveBeenCalledOnce();
  });
});

// ─── Fee Estimate ───────────────────────────────────────────────────

describe("AddKeyDialog — fee estimate", () => {
  it("shows estimated fee when provided", () => {
    setup({ estimatedFee: 50000 });
    expect(screen.getByText(/estimated fee/i)).toBeInTheDocument();
    expect(screen.getByText(/0\.00050000/)).toBeInTheDocument();
  });

  it("does not show fee section when not provided", () => {
    setup({ estimatedFee: null });
    expect(screen.queryByText(/estimated fee/i)).not.toBeInTheDocument();
  });
});

// ─── Default Config ─────────────────────────────────────────────────

describe("AddKeyDialog — default configuration", () => {
  it("respects defaultPurpose prop", () => {
    setup({ defaultPurpose: "ENCRYPTION" });
    const purposeTrigger = screen.getByLabelText("Key purpose");
    expect(purposeTrigger).toHaveTextContent("Encryption");
  });

  it("pre-fills contract bounds from defaultContractBounds", () => {
    setup({
      defaultContractBounds: {
        contractId: "ABC123",
        documentTypeName: "profile",
      },
    });
    expect(screen.getByLabelText("Contract ID")).toHaveValue("ABC123");
    expect(screen.getByLabelText("Document type name")).toHaveValue("profile");
  });
});

// ─── Accessibility ──────────────────────────────────────────────────

describe("AddKeyDialog — accessibility", () => {
  it("has form role with label", () => {
    setup();
    expect(
      screen.getByRole("form", { name: /add key form/i }),
    ).toBeInTheDocument();
  });

  it("labels and ids are associated", () => {
    setup();
    expect(screen.getByLabelText("Key purpose")).toBeInTheDocument();
    expect(screen.getByLabelText("Security level")).toBeInTheDocument();
    expect(screen.getByLabelText("Key type")).toBeInTheDocument();
    expect(screen.getByLabelText("Private key hex")).toBeInTheDocument();
  });

  it("validation errors have alert role", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("accepts className prop", () => {
    const { container } = setup({ className: "test-class" });
    // Note: in success state the className is applied to the wrapper too
    expect(container.querySelector(".test-class")).toBeInTheDocument();
  });
});

// ─── Wallet unlock gate ──────────────────────────────────────────────

describe("AddKeyDialog — wallet unlock gate", () => {
  it("shows wallet locked warning when walletLocked is true", () => {
    setup({ walletLocked: true });
    expect(screen.getByTestId("wallet-locked-gate")).toBeInTheDocument();
    expect(
      screen.getByText(/wallet is locked/i),
    ).toBeInTheDocument();
  });

  it("shows unlock button when onRequestUnlock is provided", () => {
    setup({ walletLocked: true, onRequestUnlock: vi.fn() });
    expect(
      screen.getByRole("button", { name: /unlock wallet/i }),
    ).toBeInTheDocument();
  });

  it("calls onRequestUnlock when unlock button clicked", async () => {
    const onRequestUnlock = vi.fn();
    const { user } = setup({ walletLocked: true, onRequestUnlock });
    await user.click(
      screen.getByRole("button", { name: /unlock wallet/i }),
    );
    expect(onRequestUnlock).toHaveBeenCalledOnce();
  });

  it("does not show wallet locked warning when walletLocked is false", () => {
    setup({ walletLocked: false });
    expect(screen.queryByTestId("wallet-locked-gate")).not.toBeInTheDocument();
  });

  it("disables add key button when wallet is locked", () => {
    setup({ walletLocked: true });
    expect(
      screen.getByRole("button", { name: /add key$/i }),
    ).toBeDisabled();
  });
});
