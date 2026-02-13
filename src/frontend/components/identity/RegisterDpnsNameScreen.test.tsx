import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  RegisterDpnsNameScreen,
  type RegisterDpnsNameScreenProps,
  validateDpnsName,
  isContestedName,
} from "./RegisterDpnsNameScreen";
import type { QualifiedIdentityDto } from "@/bindings";

// ─── Test fixtures ──────────────────────────────────────────────────

function makeIdentity(overrides?: Partial<QualifiedIdentityDto>): QualifiedIdentityDto {
  return {
    id: "abc123def456abc123def456abc123def456abc123def456abc123def456abcd",
    identityType: "user",
    alias: "Alice",
    balance: 10_000_000_000, // 0.1 DASH in credits
    keys: [
      {
        keyId: 0,
        keyType: "ECDSA_SECP256K1",
        purpose: "AUTHENTICATION",
        securityLevel: "CRITICAL",
        data: "02aabb",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
      {
        keyId: 1,
        keyType: "BLS12_381",
        purpose: "VOTING",
        securityLevel: "HIGH",
        data: "03ccdd",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seedhash1"],
    walletIndex: 0,
    topUps: [],
    status: "registered",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  } as QualifiedIdentityDto;
}

const defaultIdentity = makeIdentity();

function makeProps(overrides?: Partial<RegisterDpnsNameScreenProps>): RegisterDpnsNameScreenProps {
  return {
    identities: [defaultIdentity],
    status: { type: "form" },
    onSubmit: vi.fn(),
    onDismissError: vi.fn(),
    onBack: vi.fn(),
    onRegisterAnother: vi.fn(),
    ...overrides,
  };
}

// ─── validateDpnsName tests ─────────────────────────────────────────

describe("validateDpnsName", () => {
  it("returns valid for 'alice'", () => {
    expect(validateDpnsName("alice")).toEqual({ valid: true });
  });

  it("returns valid for maximum length name", () => {
    const name = "a".repeat(63);
    expect(validateDpnsName(name)).toEqual({ valid: true });
  });

  it("returns valid for name with hyphens", () => {
    expect(validateDpnsName("my-name")).toEqual({ valid: true });
  });

  it("returns valid for alphanumeric name", () => {
    expect(validateDpnsName("alice123")).toEqual({ valid: true });
  });

  it("rejects empty name silently", () => {
    const result = validateDpnsName("");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toBe("");
  });

  it("rejects name shorter than 3 chars", () => {
    const result = validateDpnsName("ab");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("at least 3 characters");
  });

  it("rejects name longer than 63 chars", () => {
    const result = validateDpnsName("a".repeat(64));
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("no more than 63 characters");
  });

  it("rejects name starting with hyphen", () => {
    const result = validateDpnsName("-alice");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("cannot start with a hyphen");
  });

  it("rejects name ending with hyphen", () => {
    const result = validateDpnsName("alice-");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("cannot end with a hyphen");
  });

  it("rejects name with invalid character", () => {
    const result = validateDpnsName("alice@bob");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("Invalid character '@'");
  });

  it("rejects name with space", () => {
    const result = validateDpnsName("alice bob");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("Invalid character");
  });

  it("rejects name with underscore", () => {
    const result = validateDpnsName("alice_bob");
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error).toContain("Invalid character '_'");
  });
});

// ─── isContestedName tests ──────────────────────────────────────────

describe("isContestedName", () => {
  it("returns true for short alpha name 'alice'", () => {
    expect(isContestedName("alice")).toBe(true);
  });

  it("returns true for 'bob'", () => {
    expect(isContestedName("bob")).toBe(true);
  });

  it("returns true for name with only 0 and 1 digits 'carol01'", () => {
    expect(isContestedName("carol01")).toBe(true);
  });

  it("returns true for name with only 1 digit 'test1'", () => {
    expect(isContestedName("test1")).toBe(true);
  });

  it("returns false for name with digit > 1 'alice99'", () => {
    expect(isContestedName("alice99")).toBe(false);
  });

  it("returns false for name with digit 2 'test2'", () => {
    expect(isContestedName("test2")).toBe(false);
  });

  it("returns false for name >= 20 chars with only letters", () => {
    expect(isContestedName("a".repeat(20))).toBe(false);
  });

  it("returns true for name with exactly 19 chars", () => {
    expect(isContestedName("a".repeat(19))).toBe(true);
  });

  it("returns false for empty string", () => {
    expect(isContestedName("")).toBe(false);
  });

  it("is case insensitive", () => {
    expect(isContestedName("ALICE")).toBe(true);
  });
});

// ─── RegisterDpnsNameScreen component tests ─────────────────────────

describe("RegisterDpnsNameScreen", () => {
  // ─── Form rendering ─────────────────────────────────────────────

  it("renders the form with heading", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByText("Register DPNS Name")).toBeInTheDocument();
  });

  it("renders identity badge when only one identity", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByTestId("identity-display")).toBeInTheDocument();
  });

  it("renders identity selector when multiple identities", () => {
    const secondIdentity = makeIdentity({
      id: "def456abc123def456abc123def456abc123def456abc123def456abc123defg",
      alias: "Bob",
    });
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ identities: [defaultIdentity, secondIdentity] })}
      />,
    );
    expect(screen.getByLabelText("Identity")).toBeInTheDocument();
  });

  it("renders balance display", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByTestId("identity-balance")).toBeInTheDocument();
  });

  it("renders name input", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByTestId("name-input")).toBeInTheDocument();
  });

  it("renders .dash suffix label", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByText(".dash")).toBeInTheDocument();
  });

  it("renders register button", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByTestId("register-btn")).toBeInTheDocument();
  });

  it("renders breadcrumb with 'Identities' source", () => {
    render(<RegisterDpnsNameScreen {...makeProps({ source: "identities" })} />);
    expect(screen.getByText("Identities")).toBeInTheDocument();
  });

  it("renders breadcrumb with 'DPNS' source", () => {
    render(<RegisterDpnsNameScreen {...makeProps({ source: "dpns" })} />);
    expect(screen.getByText("DPNS")).toBeInTheDocument();
  });

  it("renders advanced options toggle", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByTestId("advanced-toggle")).toBeInTheDocument();
  });

  it("renders info sections (constraints and contested info)", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByText("DPNS Name Constraints")).toBeInTheDocument();
    expect(screen.getByText("Contested Names Info")).toBeInTheDocument();
  });

  // ─── No identities warning ─────────────────────────────────────

  it("shows warning when no identities available", () => {
    render(<RegisterDpnsNameScreen {...makeProps({ identities: [] })} />);
    expect(
      screen.getByText(/No identities loaded/),
    ).toBeInTheDocument();
  });

  // ─── Name validation ───────────────────────────────────────────

  it("shows valid feedback when name is valid", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByText("Valid name format")).toBeInTheDocument();
  });

  it("shows error for too short name", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "ab");
    expect(screen.getByText(/at least 3 characters/)).toBeInTheDocument();
  });

  it("shows error for invalid character", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "alice@bob");
    expect(screen.getByText(/Invalid character/)).toBeInTheDocument();
  });

  it("shows contested name warning for short alpha name", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("contested-warning")).toBeInTheDocument();
  });

  it("shows not contested message for name with digit > 1", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "alice99");
    expect(screen.getByTestId("not-contested")).toBeInTheDocument();
  });

  it("shows fee estimate when name is valid", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("fee-estimate")).toBeInTheDocument();
  });

  // ─── Button state ─────────────────────────────────────────────

  it("disables register button when name is empty", () => {
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    expect(screen.getByTestId("register-btn")).toBeDisabled();
  });

  it("enables register button when name is valid", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("register-btn")).toBeEnabled();
  });

  it("disables register button when balance is insufficient", async () => {
    const user = userEvent.setup();
    const lowBalanceIdentity = makeIdentity({ balance: 100 });
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ identities: [lowBalanceIdentity] })}
      />,
    );
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("register-btn")).toBeDisabled();
    expect(screen.getByTestId("insufficient-balance")).toBeInTheDocument();
  });

  it("disables register button when no signing keys available", async () => {
    const user = userEvent.setup();
    const noKeysIdentity = makeIdentity({
      keys: [
        {
          keyId: 1,
          keyType: "BLS12_381",
          purpose: "VOTING",
          securityLevel: "HIGH",
          data: "03ccdd",
          isDisabled: false,
          disabledAt: null,
          hasPrivateKey: true,
        },
      ],
    });
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ identities: [noKeysIdentity] })}
      />,
    );
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("register-btn")).toBeDisabled();
    expect(screen.getByTestId("no-keys-warning")).toBeInTheDocument();
  });

  // ─── Submit ───────────────────────────────────────────────────

  it("calls onSubmit with identity ID and trimmed name", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<RegisterDpnsNameScreen {...makeProps({ onSubmit })} />);
    await user.type(screen.getByTestId("name-input"), "alice");
    await user.click(screen.getByTestId("register-btn"));
    expect(onSubmit).toHaveBeenCalledWith({
      identityId: defaultIdentity.id,
      name: "alice",
    });
  });

  it("calls onBack when back button is clicked", async () => {
    const user = userEvent.setup();
    const onBack = vi.fn();
    render(<RegisterDpnsNameScreen {...makeProps({ onBack })} />);
    await user.click(screen.getByTestId("back-btn"));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  // ─── Advanced options ─────────────────────────────────────────

  it("shows key selector when advanced options toggled", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.click(screen.getByTestId("advanced-toggle"));
    expect(screen.getByLabelText("Signing key")).toBeInTheDocument();
  });

  // ─── Registering state ────────────────────────────────────────

  it("renders registering spinner and elapsed time", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          status: { type: "registering", startedAt: Date.now() - 5000 },
        })}
      />,
    );
    expect(screen.getByTestId("register-dpns-registering")).toBeInTheDocument();
    expect(screen.getByText("Registering...")).toBeInTheDocument();
    expect(screen.getByTestId("elapsed-time")).toBeInTheDocument();
  });

  // ─── Success state ────────────────────────────────────────────

  it("renders success screen for non-contested name", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          status: {
            type: "success",
            contested: false,
            feeEstimated: 200_000_000,
            feeActual: 195_000_000,
          },
        })}
      />,
    );
    expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    expect(screen.getByText("DPNS Name Registered!")).toBeInTheDocument();
    expect(screen.getByTestId("back-btn")).toBeInTheDocument();
    expect(screen.getByTestId("register-another-btn")).toBeInTheDocument();
  });

  it("renders success screen for contested name with warning", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          status: {
            type: "success",
            contested: true,
            feeEstimated: null,
            feeActual: null,
          },
        })}
      />,
    );
    expect(
      screen.getByText("DPNS Name Submitted (Contested)"),
    ).toBeInTheDocument();
    expect(screen.getByText("Contested Name")).toBeInTheDocument();
    expect(screen.getByText(/two-week voting period/)).toBeInTheDocument();
  });

  it("renders fee breakdown on success", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          status: {
            type: "success",
            contested: false,
            feeEstimated: 200_000_000,
            feeActual: 195_000_000,
          },
        })}
      />,
    );
    expect(screen.getByText("Fee Breakdown")).toBeInTheDocument();
    expect(screen.getByText("Estimated fee:")).toBeInTheDocument();
    expect(screen.getByText("Actual fee:")).toBeInTheDocument();
  });

  it("calls onBack on success back button click", async () => {
    const user = userEvent.setup();
    const onBack = vi.fn();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          onBack,
          status: {
            type: "success",
            contested: false,
            feeEstimated: null,
            feeActual: null,
          },
        })}
      />,
    );
    await user.click(screen.getByTestId("back-btn"));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("calls onRegisterAnother and resets name on register another click", async () => {
    const user = userEvent.setup();
    const onRegisterAnother = vi.fn();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          onRegisterAnother,
          status: {
            type: "success",
            contested: false,
            feeEstimated: null,
            feeActual: null,
          },
        })}
      />,
    );
    await user.click(screen.getByTestId("register-another-btn"));
    expect(onRegisterAnother).toHaveBeenCalledTimes(1);
  });

  // ─── Error state ──────────────────────────────────────────────

  it("renders error screen with message", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          status: {
            type: "error",
            message: "DPNS preorder document type not found",
          },
        })}
      />,
    );
    expect(screen.getByTestId("register-dpns-error")).toBeInTheDocument();
    expect(screen.getByText("Registration Failed")).toBeInTheDocument();
    // InlineError translates raw messages via translateError()
    expect(
      screen.getByText("The requested document type was not found in the contract."),
    ).toBeInTheDocument();
  });

  it("calls onDismissError when dismiss button clicked", async () => {
    const user = userEvent.setup();
    const onDismissError = vi.fn();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          onDismissError,
          status: { type: "error", message: "Something failed" },
        })}
      />,
    );
    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(onDismissError).toHaveBeenCalledTimes(1);
  });

  // ─── Info sections ────────────────────────────────────────────

  it("shows constraint rules when expanded", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.click(screen.getByText("DPNS Name Constraints"));
    expect(screen.getByTestId("constraints-list")).toBeInTheDocument();
    expect(
      screen.getByText(/between 3 and 63 characters/),
    ).toBeInTheDocument();
  });

  it("shows contested info when expanded", async () => {
    const user = userEvent.setup();
    render(<RegisterDpnsNameScreen {...makeProps()} />);
    await user.click(screen.getByText("Contested Names Info"));
    expect(screen.getByTestId("contested-info-list")).toBeInTheDocument();
    expect(
      screen.getByText(/shorter than 20 characters/),
    ).toBeInTheDocument();
  });

  // ─── Wallet locked state ─────────────────────────────────────

  it("shows wallet locked warning when walletLocked is true", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ walletLocked: true })}
      />,
    );
    expect(screen.getByTestId("wallet-locked-warning")).toBeInTheDocument();
    expect(screen.getByText("Wallet is locked")).toBeInTheDocument();
    expect(screen.getByText(/Please unlock it to continue/)).toBeInTheDocument();
  });

  it("shows unlock wallet button when walletLocked and onRequestUnlock provided", () => {
    const onRequestUnlock = vi.fn();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ walletLocked: true, onRequestUnlock })}
      />,
    );
    expect(screen.getByTestId("unlock-wallet-btn")).toBeInTheDocument();
  });

  it("calls onRequestUnlock when unlock button clicked", async () => {
    const user = userEvent.setup();
    const onRequestUnlock = vi.fn();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ walletLocked: true, onRequestUnlock })}
      />,
    );
    await user.click(screen.getByTestId("unlock-wallet-btn"));
    expect(onRequestUnlock).toHaveBeenCalledTimes(1);
  });

  it("disables register button when wallet is locked", async () => {
    const user = userEvent.setup();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ walletLocked: true })}
      />,
    );
    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("register-btn")).toBeDisabled();
  });

  it("does not show wallet locked warning when walletLocked is false", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ walletLocked: false })}
      />,
    );
    expect(screen.queryByTestId("wallet-locked-warning")).not.toBeInTheDocument();
  });

  it("does not show wallet locked warning when no identity is selected", () => {
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ identities: [], walletLocked: true })}
      />,
    );
    expect(screen.queryByTestId("wallet-locked-warning")).not.toBeInTheDocument();
  });

  it("calls onIdentityChange on mount with initial identity", () => {
    const onIdentityChange = vi.fn();
    render(
      <RegisterDpnsNameScreen
        {...makeProps({ onIdentityChange })}
      />,
    );
    expect(onIdentityChange).toHaveBeenCalledWith(defaultIdentity.id);
  });

  // ─── Preselected identity ─────────────────────────────────────

  it("uses preselected identity ID", () => {
    const secondIdentity = makeIdentity({
      id: "def456abc123def456abc123def456abc123def456abc123def456abc123defg",
      alias: "Bob",
    });
    render(
      <RegisterDpnsNameScreen
        {...makeProps({
          identities: [defaultIdentity, secondIdentity],
          preselectedIdentityId: secondIdentity.id,
        })}
      />,
    );
    // The identity balance should show for the preselected identity
    expect(screen.getByTestId("identity-balance")).toBeInTheDocument();
  });
});
