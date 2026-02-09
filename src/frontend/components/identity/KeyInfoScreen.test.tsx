import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { KeyInfoScreen } from "./KeyInfoScreen";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    keyType: "ECDSA_SECP256K1",
    purpose: "AUTHENTICATION",
    securityLevel: "MASTER",
    data: "02aabbccdd",
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: false,
    ...overrides,
  };
}

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aabbccdd11223344556677889900aabb",
    identityType: "user",
    alias: "Alice",
    balance: 1_000_000_000,
    keys: [makeKey()],
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
  keyData: makeKey(),
  onBack: vi.fn(),
  onDisableKey: vi.fn(),
  onReplaceKey: vi.fn(),
  onAddPrivateKey: vi.fn(),
  onRemovePrivateKey: vi.fn(),
  onSignMessage: vi.fn().mockResolvedValue("c2lnbmVkX21lc3NhZ2VfYmFzZTY0"),
  isSubmitting: false,
  error: null,
  success: null,
  onClearError: vi.fn(),
  onClearSuccess: vi.fn(),
};

function setup(
  props: Partial<Parameters<typeof KeyInfoScreen>[0]> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <KeyInfoScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("KeyInfoScreen — header", () => {
  it("renders title and identity info", () => {
    setup();
    expect(screen.getByText("Key Info")).toBeInTheDocument();
    expect(screen.getByText(/Alice — Key #0/)).toBeInTheDocument();
  });

  it("shows truncated ID when no alias", () => {
    setup({ identity: makeIdentity({ alias: null }) });
    expect(screen.getByText(/aabbccdd…00aabb — Key #0/)).toBeInTheDocument();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /back to key list/i }));
    expect(props.onBack).toHaveBeenCalledOnce();
  });

  it("hides back button when onBack not provided", () => {
    setup({ onBack: undefined });
    expect(
      screen.queryByRole("button", { name: /back to key list/i }),
    ).not.toBeInTheDocument();
  });
});

// ─── Key Metadata ───────────────────────────────────────────────────

describe("KeyInfoScreen — key metadata", () => {
  it("displays key ID", () => {
    setup({ keyData: makeKey({ keyId: 42 }) });
    expect(screen.getByText("42")).toBeInTheDocument();
  });

  it("displays purpose in title case", () => {
    setup({ keyData: makeKey({ purpose: "AUTHENTICATION" }) });
    expect(screen.getByText("Authentication")).toBeInTheDocument();
  });

  it("displays security level", () => {
    setup({ keyData: makeKey({ securityLevel: "CRITICAL" }) });
    expect(screen.getByText("Critical")).toBeInTheDocument();
  });

  it("displays key type label", () => {
    setup({ keyData: makeKey({ keyType: "ECDSA_SECP256K1" }) });
    expect(screen.getByText("ECDSA secp256k1")).toBeInTheDocument();
  });

  it("displays BLS key type", () => {
    setup({ keyData: makeKey({ keyType: "BLS12_381" }) });
    expect(screen.getByText("BLS 12-381")).toBeInTheDocument();
  });

  it("shows Active badge for non-disabled key", () => {
    setup({ keyData: makeKey({ isDisabled: false }) });
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  it("shows Disabled badge for disabled key", () => {
    setup({ keyData: makeKey({ isDisabled: true }) });
    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });

  it("shows disabled timestamp when key is disabled", () => {
    setup({
      keyData: makeKey({ isDisabled: true, disabledAt: 1700000000 }),
    });
    expect(screen.getByText("Disabled At")).toBeInTheDocument();
  });
});

// ─── Public Key ─────────────────────────────────────────────────────

describe("KeyInfoScreen — public key", () => {
  it("displays public key hex", () => {
    setup({ keyData: makeKey({ data: "02aabbccdd" }) });
    expect(screen.getByText("02aabbccdd")).toBeInTheDocument();
  });

  it("displays Hex and Base64 labels", () => {
    setup();
    expect(screen.getByText("Hex")).toBeInTheDocument();
    expect(screen.getByText("Base64")).toBeInTheDocument();
  });

  it("renders copy buttons for public key", () => {
    setup();
    const copyButtons = screen.getAllByRole("button", {
      name: /copy to clipboard/i,
    });
    expect(copyButtons.length).toBeGreaterThanOrEqual(2);
  });
});

// ─── Private Key — No Key ───────────────────────────────────────────

describe("KeyInfoScreen — private key (not available)", () => {
  it("shows 'No private key' message when hasPrivateKey is false", () => {
    setup({ keyData: makeKey({ hasPrivateKey: false }) });
    expect(
      screen.getByText("No private key in local storage"),
    ).toBeInTheDocument();
  });

  it("shows private key hex input", () => {
    setup({ keyData: makeKey({ hasPrivateKey: false }) });
    expect(
      screen.getByLabelText("Private key hex input"),
    ).toBeInTheDocument();
  });

  it("shows Add button for adding private key", () => {
    setup({ keyData: makeKey({ hasPrivateKey: false }) });
    expect(screen.getByRole("button", { name: "Add" })).toBeInTheDocument();
  });

  it("Add button is disabled when input is empty", () => {
    setup({ keyData: makeKey({ hasPrivateKey: false }) });
    expect(screen.getByRole("button", { name: "Add" })).toBeDisabled();
  });

  it("calls onAddPrivateKey with valid hex", async () => {
    const { user, props } = setup({
      keyData: makeKey({ keyId: 5, hasPrivateKey: false }),
    });
    const input = screen.getByLabelText("Private key hex input");
    await user.type(input, "a".repeat(64));
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(props.onAddPrivateKey).toHaveBeenCalledWith(5, "a".repeat(64));
  });

  it("shows error for invalid hex", async () => {
    const { user } = setup({
      keyData: makeKey({ hasPrivateKey: false }),
    });
    const input = screen.getByLabelText("Private key hex input");
    await user.type(input, "invalidhexXYZ");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(screen.getByRole("alert")).toHaveTextContent(/hex string/i);
  });

  it("shows error for wrong length", async () => {
    const { user } = setup({
      keyData: makeKey({ hasPrivateKey: false }),
    });
    const input = screen.getByLabelText("Private key hex input");
    await user.type(input, "aabb");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(screen.getByRole("alert")).toHaveTextContent(/32 bytes/i);
  });
});

// ─── Private Key — Available ────────────────────────────────────────

describe("KeyInfoScreen — private key (available)", () => {
  it("shows 'Private key is available' message", () => {
    setup({ keyData: makeKey({ hasPrivateKey: true }) });
    expect(
      screen.getByText("Private key is available in local storage"),
    ).toBeInTheDocument();
  });

  it("shows Remove Private Key button", () => {
    setup({ keyData: makeKey({ hasPrivateKey: true }) });
    expect(
      screen.getByRole("button", { name: /remove private key/i }),
    ).toBeInTheDocument();
  });

  it("shows confirmation dialog on remove click", async () => {
    const { user } = setup({ keyData: makeKey({ hasPrivateKey: true }) });
    await user.click(
      screen.getByRole("button", { name: /remove private key/i }),
    );
    expect(screen.getByText("Remove Private Key")).toBeInTheDocument();
    expect(
      screen.getByText(/are you sure you want to remove the private key/i),
    ).toBeInTheDocument();
  });

  it("calls onRemovePrivateKey after confirmation", async () => {
    const { user, props } = setup({
      keyData: makeKey({ keyId: 3, hasPrivateKey: true }),
    });
    await user.click(
      screen.getByRole("button", { name: /remove private key/i }),
    );
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect(props.onRemovePrivateKey).toHaveBeenCalledWith(3);
  });

  it("does not show private key input when key is available", () => {
    setup({ keyData: makeKey({ hasPrivateKey: true }) });
    expect(
      screen.queryByLabelText("Private key hex input"),
    ).not.toBeInTheDocument();
  });
});

// ─── Sign Message ───────────────────────────────────────────────────

describe("KeyInfoScreen — sign message", () => {
  it("shows sign message section when private key available", () => {
    setup({ keyData: makeKey({ hasPrivateKey: true }) });
    expect(
      screen.getByRole("region", { name: /sign message/i }),
    ).toBeInTheDocument();
  });

  it("does not show sign message section when no private key", () => {
    setup({ keyData: makeKey({ hasPrivateKey: false }) });
    expect(
      screen.queryByRole("region", { name: /sign message/i }),
    ).not.toBeInTheDocument();
  });

  it("Sign Message button is disabled when input is empty", () => {
    setup({ keyData: makeKey({ hasPrivateKey: true }) });
    expect(
      screen.getByRole("button", { name: /sign message/i }),
    ).toBeDisabled();
  });

  it("calls onSignMessage with message text", async () => {
    const { user, props } = setup({
      keyData: makeKey({ keyId: 2, hasPrivateKey: true }),
    });
    await user.type(
      screen.getByLabelText("Message to sign"),
      "Hello, world!",
    );
    await user.click(screen.getByRole("button", { name: /sign message/i }));
    expect(props.onSignMessage).toHaveBeenCalledWith(2, "Hello, world!");
  });

  it("displays signed message after successful signing", async () => {
    const { user } = setup({
      keyData: makeKey({ hasPrivateKey: true }),
    });
    await user.type(
      screen.getByLabelText("Message to sign"),
      "test",
    );
    await user.click(screen.getByRole("button", { name: /sign message/i }));
    // The mock returns "c2lnbmVkX21lc3NhZ2VfYmFzZTY0"
    expect(
      await screen.findByText("c2lnbmVkX21lc3NhZ2VfYmFzZTY0"),
    ).toBeInTheDocument();
  });

  it("displays error on signing failure", async () => {
    const { user } = setup({
      keyData: makeKey({ hasPrivateKey: true }),
      onSignMessage: vi.fn().mockRejectedValue(new Error("Sign failed")),
    });
    await user.type(
      screen.getByLabelText("Message to sign"),
      "test",
    );
    await user.click(screen.getByRole("button", { name: /sign message/i }));
    expect(
      await screen.findByText("Sign failed"),
    ).toBeInTheDocument();
  });
});

// ─── Key Actions — Disable ──────────────────────────────────────────

describe("KeyInfoScreen — disable key", () => {
  it("shows disable button for non-master, non-disabled key", () => {
    setup({
      keyData: makeKey({
        securityLevel: "HIGH",
        isDisabled: false,
      }),
    });
    expect(
      screen.getByRole("button", { name: /disable key on platform/i }),
    ).toBeInTheDocument();
  });

  it("does not show disable button for master key", () => {
    setup({
      keyData: makeKey({ securityLevel: "MASTER", isDisabled: false }),
    });
    expect(
      screen.queryByRole("button", { name: /disable key on platform/i }),
    ).not.toBeInTheDocument();
  });

  it("does not show disable button for already disabled key", () => {
    setup({
      keyData: makeKey({ securityLevel: "HIGH", isDisabled: true }),
    });
    expect(
      screen.queryByRole("button", { name: /disable key on platform/i }),
    ).not.toBeInTheDocument();
  });

  it("shows confirmation dialog on disable click", async () => {
    const { user } = setup({
      keyData: makeKey({
        keyId: 5,
        securityLevel: "HIGH",
        purpose: "AUTHENTICATION",
      }),
    });
    await user.click(
      screen.getByRole("button", { name: /disable key on platform/i }),
    );
    expect(screen.getByText("Disable Key")).toBeInTheDocument();
    expect(
      screen.getByText(/are you sure you want to disable key #5/i),
    ).toBeInTheDocument();
  });

  it("calls onDisableKey after confirmation", async () => {
    const { user, props } = setup({
      keyData: makeKey({ keyId: 5, securityLevel: "HIGH" }),
    });
    await user.click(
      screen.getByRole("button", { name: /disable key on platform/i }),
    );
    await user.click(screen.getByRole("button", { name: "Disable" }));
    expect(props.onDisableKey).toHaveBeenCalledWith(5);
  });

  it("shows loading state when submitting", () => {
    setup({
      keyData: makeKey({ securityLevel: "HIGH" }),
      isSubmitting: true,
    });
    expect(screen.getByText("Disabling…")).toBeInTheDocument();
  });
});

// ─── Key Actions — Replace Master ───────────────────────────────────

describe("KeyInfoScreen — replace master key", () => {
  // Mock crypto.getRandomValues for deterministic tests
  const mockRandomValues = vi.spyOn(crypto, "getRandomValues");

  beforeEach(() => {
    // Return deterministic "random" bytes: 0x01, 0x02, ..., 0x20
    mockRandomValues.mockImplementation((arr: ArrayBufferView) => {
      const bytes = new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength);
      for (let i = 0; i < bytes.length; i++) {
        bytes[i] = (i + 1) & 0xff;
      }
      return arr;
    });
  });

  it("shows replace button for master key", () => {
    setup({
      keyData: makeKey({ securityLevel: "MASTER" }),
    });
    expect(
      screen.getByRole("button", { name: /replace master key/i }),
    ).toBeInTheDocument();
  });

  it("does not show replace button for non-master key", () => {
    setup({
      keyData: makeKey({ securityLevel: "HIGH" }),
    });
    expect(
      screen.queryByRole("button", { name: /replace master key/i }),
    ).not.toBeInTheDocument();
  });

  it("shows replace dialog with key type selector and generated key", async () => {
    const { user } = setup({
      keyData: makeKey({ keyId: 0, securityLevel: "MASTER", keyType: "ECDSA_SECP256K1" }),
    });
    await user.click(
      screen.getByRole("button", { name: /replace master key/i }),
    );
    // Dialog title and warning message
    expect(screen.getByText("Replace Master Key", { selector: "[data-slot='dialog-title']" })).toBeInTheDocument();
    expect(
      screen.getByText(/make sure to save the new private key/i),
    ).toBeInTheDocument();
    // Old key info
    expect(screen.getByText(/Old Key ID: 0/)).toBeInTheDocument();
    // Key type selector label
    expect(screen.getByText("New Key Type")).toBeInTheDocument();
    // Generated private key (deterministic from mock)
    const expectedHex = Array.from({ length: 32 }, (_, i) =>
      ((i + 1) & 0xff).toString(16).padStart(2, "0"),
    ).join("");
    expect(screen.getByLabelText("Generated private key")).toHaveValue(expectedHex);
    // Regenerate button
    expect(screen.getByRole("button", { name: /regenerate/i })).toBeInTheDocument();
    // Cancel and confirm buttons
    expect(screen.getByRole("button", { name: /^cancel$/i })).toBeInTheDocument();
  });

  it("calls onReplaceKey with generated key after confirmation", async () => {
    const { user, props } = setup({
      keyData: makeKey({ keyId: 0, securityLevel: "MASTER" }),
    });
    await user.click(
      screen.getByRole("button", { name: /replace master key/i }),
    );
    // The generated key from our mock
    const expectedHex = Array.from({ length: 32 }, (_, i) =>
      ((i + 1) & 0xff).toString(16).padStart(2, "0"),
    ).join("");
    // Click the "Replace Master Key" button in the dialog
    const replaceButtons = screen.getAllByRole("button", { name: /replace master key/i });
    // The dialog confirm button is the last one (the first is the trigger button)
    await user.click(replaceButtons[replaceButtons.length - 1]);
    expect(props.onReplaceKey).toHaveBeenCalledWith(0, "ECDSA_SECP256K1", expectedHex);
  });

  it("regenerates key on Regenerate button click", async () => {
    let callCount = 0;
    mockRandomValues.mockImplementation((arr: ArrayBufferView) => {
      callCount++;
      const bytes = new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength);
      for (let i = 0; i < bytes.length; i++) {
        bytes[i] = ((i + callCount) & 0xff);
      }
      return arr;
    });

    const { user } = setup({
      keyData: makeKey({ keyId: 0, securityLevel: "MASTER" }),
    });
    await user.click(
      screen.getByRole("button", { name: /replace master key/i }),
    );
    const input = screen.getByLabelText("Generated private key");
    const firstValue = input.getAttribute("value");
    await user.click(screen.getByRole("button", { name: /regenerate/i }));
    const secondValue = input.getAttribute("value");
    expect(firstValue).not.toBe(secondValue);
  });

  it("shows loading state when submitting", () => {
    setup({
      keyData: makeKey({ securityLevel: "MASTER" }),
      isSubmitting: true,
    });
    expect(screen.getByText("Replacing…")).toBeInTheDocument();
  });
});

// ─── Error / Success Messages ───────────────────────────────────────

describe("KeyInfoScreen — status messages", () => {
  it("renders error message", () => {
    setup({ error: "Something went wrong" });
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
  });

  it("calls onClearError when dismiss clicked", async () => {
    const { user, props } = setup({ error: "Oops" });
    await user.click(
      screen.getByRole("button", { name: /dismiss error/i }),
    );
    expect(props.onClearError).toHaveBeenCalledOnce();
  });

  it("renders success message", () => {
    setup({ success: "Key disabled successfully" });
    expect(
      screen.getByText("Key disabled successfully"),
    ).toBeInTheDocument();
  });

  it("calls onClearSuccess when dismiss clicked", async () => {
    const { user, props } = setup({ success: "Done" });
    await user.click(
      screen.getByRole("button", { name: /dismiss success/i }),
    );
    expect(props.onClearSuccess).toHaveBeenCalledOnce();
  });
});

// ─── Accessibility ──────────────────────────────────────────────────

describe("KeyInfoScreen — accessibility", () => {
  it("has region role with label", () => {
    setup();
    expect(
      screen.getByRole("region", { name: "Key information" }),
    ).toBeInTheDocument();
  });

  it("has labeled sections", () => {
    setup({ keyData: makeKey({ hasPrivateKey: true }) });
    expect(
      screen.getByRole("region", { name: /key metadata/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: /public key/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: /private key/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: /sign message/i }),
    ).toBeInTheDocument();
  });

  it("error messages have alert role", () => {
    setup({ error: "Test error" });
    expect(screen.getByRole("alert")).toHaveTextContent("Test error");
  });

  it("success messages have status role", () => {
    setup({ success: "Test success" });
    expect(screen.getByRole("status")).toHaveTextContent("Test success");
  });
});

// ─── Wallet unlock gate ──────────────────────────────────────────────

describe("KeyInfoScreen — wallet unlock gate", () => {
  const nonMasterKey = makeKey({
    keyId: 2,
    purpose: "AUTHENTICATION",
    securityLevel: "HIGH",
    hasPrivateKey: true,
  });

  it("shows wallet locked warning in key actions when walletLocked is true", () => {
    setup({
      keyData: nonMasterKey,
      walletLocked: true,
    });
    expect(screen.getByTestId("wallet-locked-gate")).toBeInTheDocument();
    expect(
      screen.getByText(/wallet is locked/i),
    ).toBeInTheDocument();
  });

  it("shows unlock button when onRequestUnlock is provided", () => {
    setup({
      keyData: nonMasterKey,
      walletLocked: true,
      onRequestUnlock: vi.fn(),
    });
    expect(
      screen.getByRole("button", { name: /unlock wallet/i }),
    ).toBeInTheDocument();
  });

  it("calls onRequestUnlock when unlock button clicked", async () => {
    const onRequestUnlock = vi.fn();
    const { user } = setup({
      keyData: nonMasterKey,
      walletLocked: true,
      onRequestUnlock,
    });
    await user.click(
      screen.getByRole("button", { name: /unlock wallet/i }),
    );
    expect(onRequestUnlock).toHaveBeenCalledOnce();
  });

  it("does not show wallet locked warning when walletLocked is false", () => {
    setup({
      keyData: nonMasterKey,
      walletLocked: false,
    });
    expect(screen.queryByTestId("wallet-locked-gate")).not.toBeInTheDocument();
  });

  it("disables disable-key button when wallet is locked", () => {
    setup({
      keyData: nonMasterKey,
      walletLocked: true,
    });
    expect(
      screen.getByRole("button", { name: /disable key on platform/i }),
    ).toBeDisabled();
  });

  it("disables replace-master-key button when wallet is locked", () => {
    setup({
      keyData: makeKey({ keyId: 0, securityLevel: "MASTER" }),
      walletLocked: true,
    });
    expect(
      screen.getByRole("button", { name: /replace master key/i }),
    ).toBeDisabled();
  });
});
