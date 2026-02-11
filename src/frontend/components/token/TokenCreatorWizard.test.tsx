import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  TokenCreatorWizard,
  BasicInfoStep,
  SimpleInfoForm,
  validateBasicInfo,
  validateSimpleMode,
  applyPresetToControlRules,
  createDefaultBasicInfo,
  TOKEN_LANGUAGES,
  TOKEN_PRESETS,
  WIZARD_STEPS,
} from "./TokenCreatorWizard";
import type { BasicInfoState } from "./TokenCreatorWizard";

// ─── Mocks ──────────────────────────────────────────────────────────

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      identities: [
        {
          id: "id-abc123",
          alias: "TestIdentity",
          balance: 1000000000,
          keys: [
            {
              keyId: 1,
              purpose: "AUTHENTICATION",
              securityLevel: "HIGH",
              keyType: "ECDSA_SECP256K1",
              isDisabled: false,
            },
          ],
          associatedWalletHashes: ["seed-hash-1"],
          dpnsNames: [],
          identityType: "User",
          walletIndex: 0,
          topUps: [],
          status: "NotInPlatform",
        },
      ],
      loadIdentities: vi.fn(),
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
      loadWallets: vi.fn(),
    };
    return sel ? sel(s) : s;
  },
}));

vi.mock("@/stores/tokenStore", () => ({
  useTokenStore: () => ({
    loadMyTokenBalances: vi.fn().mockResolvedValue(null),
  }),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

vi.mock("@/components/shared/WalletUnlockDialog", () => ({
  WalletUnlockDialog: () => null,
}));

// ─── Helpers ────────────────────────────────────────────────────────

function validBasicInfo(overrides: Partial<BasicInfoState> = {}): BasicInfoState {
  return {
    ...createDefaultBasicInfo(),
    names: [{ singular: "TestToken", plural: "TestTokens", languageCode: "en" }],
    baseSupply: "1000000",
    ...overrides,
  };
}

// ─── validateBasicInfo unit tests ───────────────────────────────────

describe("validateBasicInfo", () => {
  it("returns valid for a complete, correct state", () => {
    const result = validateBasicInfo(validBasicInfo());
    expect(result.valid).toBe(true);
    expect(Object.keys(result.errors)).toHaveLength(0);
  });

  it("requires token name", () => {
    const result = validateBasicInfo(
      validBasicInfo({ names: [{ singular: "", plural: "", languageCode: "en" }] }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.tokenName).toBe("Token name is required");
  });

  it("requires token name at least 3 chars", () => {
    const result = validateBasicInfo(
      validBasicInfo({ names: [{ singular: "ab", plural: "abs", languageCode: "en" }] }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.tokenName).toBe("Token name must be at least 3 characters");
  });

  it("requires token name at most 50 chars", () => {
    const result = validateBasicInfo(
      validBasicInfo({
        names: [{ singular: "a".repeat(51), plural: "abs", languageCode: "en" }],
      }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.tokenName).toBe("Token name must be at most 50 characters");
  });

  it("requires base supply", () => {
    const result = validateBasicInfo(validBasicInfo({ baseSupply: "" }));
    expect(result.valid).toBe(false);
    expect(result.errors.baseSupply).toBe("Base supply is required");
  });

  it("rejects non-numeric base supply", () => {
    const result = validateBasicInfo(validBasicInfo({ baseSupply: "abc" }));
    expect(result.valid).toBe(false);
    expect(result.errors.baseSupply).toBe("Base supply must be a valid number");
  });

  it("rejects zero base supply", () => {
    const result = validateBasicInfo(validBasicInfo({ baseSupply: "0" }));
    expect(result.valid).toBe(false);
    expect(result.errors.baseSupply).toBe("Base supply must be greater than 0");
  });

  it("rejects non-numeric max supply", () => {
    const result = validateBasicInfo(validBasicInfo({ maxSupply: "xyz" }));
    expect(result.valid).toBe(false);
    expect(result.errors.maxSupply).toBe("Max supply must be a valid number");
  });

  it("allows empty max supply (unlimited)", () => {
    const result = validateBasicInfo(validBasicInfo({ maxSupply: "" }));
    expect(result.valid).toBe(true);
  });

  it("allows zero max supply (unlimited)", () => {
    const result = validateBasicInfo(validBasicInfo({ maxSupply: "0" }));
    expect(result.valid).toBe(true);
  });

  it("rejects non-numeric decimals", () => {
    const result = validateBasicInfo(validBasicInfo({ decimals: "abc" }));
    expect(result.valid).toBe(false);
    expect(result.errors.decimals).toBe("Decimals must be a whole number");
  });

  it("rejects decimals > 99", () => {
    const result = validateBasicInfo(validBasicInfo({ decimals: "100" }));
    expect(result.valid).toBe(false);
    expect(result.errors.decimals).toBe("Decimals must be between 0 and 99");
  });

  it("allows decimals = 0 (non-fractional)", () => {
    const result = validateBasicInfo(validBasicInfo({ decimals: "0" }));
    expect(result.valid).toBe(true);
  });

  it("rejects description over 100 chars", () => {
    const result = validateBasicInfo(
      validBasicInfo({ description: "x".repeat(101) }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.description).toBe(
      "Description must be at most 100 characters",
    );
  });

  it("allows description up to 100 chars", () => {
    const result = validateBasicInfo(
      validBasicInfo({ description: "x".repeat(100) }),
    );
    expect(result.valid).toBe(true);
  });

  it("detects duplicate languages", () => {
    const result = validateBasicInfo(
      validBasicInfo({
        names: [
          { singular: "Token", plural: "Tokens", languageCode: "en" },
          { singular: "Token", plural: "Tokens", languageCode: "en" },
        ],
      }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.language_1).toBe("Duplicate language");
  });

  it("validates additional name lengths", () => {
    const result = validateBasicInfo(
      validBasicInfo({
        names: [
          { singular: "Token", plural: "Tokens", languageCode: "en" },
          { singular: "ab", plural: "Tokens", languageCode: "fr" },
        ],
      }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.tokenName_1).toContain("between 3 and 50");
  });

  it("rejects keywords shorter than 3 chars", () => {
    const result = validateBasicInfo(
      validBasicInfo({ contractKeywords: "ok, ab" }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.contractKeywords).toContain("between 3 and 50");
  });

  it("rejects duplicate keywords (case-insensitive)", () => {
    const result = validateBasicInfo(
      validBasicInfo({ contractKeywords: "gaming, Gaming" }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.contractKeywords).toContain("Duplicate keyword");
  });

  it("accepts valid keywords", () => {
    const result = validateBasicInfo(
      validBasicInfo({ contractKeywords: "gaming, reward, loyalty" }),
    );
    expect(result.valid).toBe(true);
  });
});

// ─── createDefaultBasicInfo ─────────────────────────────────────────

describe("createDefaultBasicInfo", () => {
  it("returns English as default language", () => {
    const info = createDefaultBasicInfo();
    expect(info.names).toHaveLength(1);
    expect(info.names[0].languageCode).toBe("en");
  });

  it("returns decimals = 8 by default", () => {
    expect(createDefaultBasicInfo().decimals).toBe("8");
  });

  it("returns all boolean flags as false by default", () => {
    const info = createDefaultBasicInfo();
    expect(info.shouldCapitalize).toBe(false);
    expect(info.startPaused).toBe(false);
    expect(info.allowTransfersToFrozen).toBe(false);
  });

  it("returns no preset selected", () => {
    expect(createDefaultBasicInfo().preset).toBeNull();
  });
});

// ─── TOKEN_LANGUAGES ────────────────────────────────────────────────

describe("TOKEN_LANGUAGES", () => {
  it("has English as the first entry", () => {
    expect(TOKEN_LANGUAGES[0].code).toBe("en");
    expect(TOKEN_LANGUAGES[0].label).toBe("English");
  });

  it("has 52 languages", () => {
    expect(TOKEN_LANGUAGES.length).toBe(52);
  });

  it("has unique language codes", () => {
    const codes = TOKEN_LANGUAGES.map((l) => l.code);
    expect(new Set(codes).size).toBe(codes.length);
  });
});

// ─── TOKEN_PRESETS ──────────────────────────────────────────────────

describe("TOKEN_PRESETS", () => {
  it("has 5 presets", () => {
    expect(TOKEN_PRESETS).toHaveLength(5);
  });

  it("has unique values", () => {
    const values = TOKEN_PRESETS.map((p) => p.value);
    expect(new Set(values).size).toBe(values.length);
  });
});

// ─── WIZARD_STEPS ───────────────────────────────────────────────────

describe("WIZARD_STEPS", () => {
  it("has 7 steps", () => {
    expect(WIZARD_STEPS).toHaveLength(7);
  });

  it("starts with basicInfo", () => {
    expect(WIZARD_STEPS[0].key).toBe("basicInfo");
  });

  it("ends with review", () => {
    expect(WIZARD_STEPS[WIZARD_STEPS.length - 1].key).toBe("review");
  });
});

// ─── BasicInfoStep component tests ──────────────────────────────────

describe("BasicInfoStep", () => {
  let onChange: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onChange = vi.fn();
  });

  it("renders the token name input", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("token-name-singular-0")).toBeInTheDocument();
  });

  it("renders the plural name input", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("token-name-plural-0")).toBeInTheDocument();
  });

  it("renders description input with character counter", () => {
    render(
      <BasicInfoStep
        state={{ ...createDefaultBasicInfo(), description: "Hello" }}
        onChange={onChange}
      />,
    );
    expect(screen.getByTestId("token-description")).toBeInTheDocument();
    expect(screen.getByText("5/100")).toBeInTheDocument();
  });

  it("renders base supply input", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("base-supply")).toBeInTheDocument();
  });

  it("renders max supply input", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("max-supply")).toBeInTheDocument();
  });

  it("renders preset selector", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("token-preset-select")).toBeInTheDocument();
  });

  it("renders language selector for first entry", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("language-select-0")).toBeInTheDocument();
  });

  it("renders Add Language button", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("add-language")).toBeInTheDocument();
  });

  it("does not render remove button for the first (English) language", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.queryByTestId("remove-language-0")).not.toBeInTheDocument();
  });

  it("calls onChange when typing in token name", async () => {
    const user = userEvent.setup();
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    await user.type(screen.getByTestId("token-name-singular-0"), "A");
    expect(onChange).toHaveBeenCalled();
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0];
    expect(lastCall.names[0].singular).toBe("A");
  });

  it("calls onChange when typing in base supply (strips non-numeric)", async () => {
    const user = userEvent.setup();
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    await user.type(screen.getByTestId("base-supply"), "abc123");
    // Only digits should be passed through
    const calls = onChange.mock.calls.map((c: BasicInfoState[]) => c[0].baseSupply);
    expect(calls.every((v: string) => /^[\d.]*$/.test(v))).toBe(true);
  });

  it("shows validation error when token name is too short", () => {
    const state = validBasicInfo({
      names: [{ singular: "ab", plural: "", languageCode: "en" }],
    });
    render(<BasicInfoStep state={state} onChange={onChange} />);
    expect(
      screen.getByText("Token name must be at least 3 characters"),
    ).toBeInTheDocument();
  });

  it("shows validation error when base supply is empty", () => {
    const state = validBasicInfo({ baseSupply: "" });
    render(<BasicInfoStep state={state} onChange={onChange} />);
    expect(screen.getByText("Base supply is required")).toBeInTheDocument();
  });

  it("shows Advanced Options toggle (collapsed by default)", () => {
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("advanced-toggle")).toBeInTheDocument();
    // Advanced fields should not be visible
    expect(screen.queryByTestId("start-paused")).not.toBeInTheDocument();
  });

  it("expands advanced options on click", async () => {
    const user = userEvent.setup();
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    await user.click(screen.getByTestId("advanced-toggle"));
    expect(screen.getByTestId("start-paused")).toBeInTheDocument();
    expect(screen.getByTestId("should-capitalize")).toBeInTheDocument();
    expect(screen.getByTestId("allow-frozen-transfers")).toBeInTheDocument();
    expect(screen.getByTestId("decimals")).toBeInTheDocument();
    expect(screen.getByTestId("contract-keywords")).toBeInTheDocument();
  });

  it("shows fractional/non-fractional label based on decimals", async () => {
    const user = userEvent.setup();
    const state = validBasicInfo({ decimals: "0" });
    render(<BasicInfoStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("advanced-toggle"));
    expect(screen.getByText(/Non-fractional token/)).toBeInTheDocument();
  });

  it("shows fractional label when decimals > 0", async () => {
    const user = userEvent.setup();
    const state = validBasicInfo({ decimals: "8" });
    render(<BasicInfoStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("advanced-toggle"));
    expect(screen.getByText(/Fractional token/)).toBeInTheDocument();
  });

  it("adds a second language entry when Add Language is clicked", async () => {
    const user = userEvent.setup();
    render(<BasicInfoStep state={createDefaultBasicInfo()} onChange={onChange} />);
    await user.click(screen.getByTestId("add-language"));
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0];
    expect(lastCall.names).toHaveLength(2);
    expect(lastCall.names[1].languageCode).not.toBe("en");
  });

  it("removes a second language entry when Remove is clicked", async () => {
    const user = userEvent.setup();
    const state = validBasicInfo({
      names: [
        { singular: "Token", plural: "Tokens", languageCode: "en" },
        { singular: "Jeton", plural: "Jetons", languageCode: "fr" },
      ],
    });
    render(<BasicInfoStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("remove-language-1"));
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0];
    expect(lastCall.names).toHaveLength(1);
  });

  it("uses plural placeholder from singular name", () => {
    const state = validBasicInfo({
      names: [{ singular: "MyCoin", plural: "", languageCode: "en" }],
    });
    render(<BasicInfoStep state={state} onChange={onChange} />);
    const pluralInput = screen.getByTestId("token-name-plural-0");
    expect(pluralInput).toHaveAttribute("placeholder", "MyCoins");
  });
});

// ─── validateSimpleMode ──────────────────────────────────────────────

describe("validateSimpleMode", () => {
  it("requires a preset to be selected", () => {
    const result = validateSimpleMode(validBasicInfo({ preset: null }));
    expect(result.valid).toBe(false);
    expect(result.errors.preset).toBe("A token preset is required");
  });

  it("passes when preset is selected", () => {
    const result = validateSimpleMode(
      validBasicInfo({ preset: "mostRestrictive" }),
    );
    expect(result.valid).toBe(true);
  });

  it("still validates base info fields", () => {
    const result = validateSimpleMode(
      validBasicInfo({ baseSupply: "", preset: "allAllowed" }),
    );
    expect(result.valid).toBe(false);
    expect(result.errors.baseSupply).toBeTruthy();
  });
});

// ─── applyPresetToControlRules ──────────────────────────────────────

describe("applyPresetToControlRules", () => {
  it("mostRestrictive sets all action takers to noOne", () => {
    const rules = applyPresetToControlRules("mostRestrictive");
    expect(rules.manualMinting.authorizedActionTaker).toBe("noOne");
    expect(rules.manualBurning.authorizedActionTaker).toBe("noOne");
    expect(rules.freeze.authorizedActionTaker).toBe("noOne");
    expect(rules.unfreeze.authorizedActionTaker).toBe("noOne");
    expect(rules.destroyFrozenFunds.authorizedActionTaker).toBe("noOne");
    expect(rules.emergencyAction.authorizedActionTaker).toBe("noOne");
    expect(rules.maxSupplyChange.authorizedActionTaker).toBe("noOne");
    expect(rules.conventionsChange.authorizedActionTaker).toBe("noOne");
    expect(rules.mainControlGroupChange).toBe("noOne");
  });

  it("onlyEmergency enables emergency action only", () => {
    const rules = applyPresetToControlRules("onlyEmergency");
    expect(rules.emergencyAction.authorizedActionTaker).toBe("contractOwner");
    expect(rules.manualMinting.authorizedActionTaker).toBe("noOne");
    expect(rules.freeze.authorizedActionTaker).toBe("noOne");
  });

  it("mintingAndBurning enables minting, burning, and emergency", () => {
    const rules = applyPresetToControlRules("mintingAndBurning");
    expect(rules.manualMinting.authorizedActionTaker).toBe("contractOwner");
    expect(rules.manualBurning.authorizedActionTaker).toBe("contractOwner");
    expect(rules.emergencyAction.authorizedActionTaker).toBe("contractOwner");
    expect(rules.freeze.authorizedActionTaker).toBe("noOne");
    expect(rules.unfreeze.authorizedActionTaker).toBe("noOne");
  });

  it("advancedActions enables mint, burn, freeze, unfreeze, emergency", () => {
    const rules = applyPresetToControlRules("advancedActions");
    expect(rules.manualMinting.authorizedActionTaker).toBe("contractOwner");
    expect(rules.manualBurning.authorizedActionTaker).toBe("contractOwner");
    expect(rules.freeze.authorizedActionTaker).toBe("contractOwner");
    expect(rules.unfreeze.authorizedActionTaker).toBe("contractOwner");
    expect(rules.emergencyAction.authorizedActionTaker).toBe("contractOwner");
    // Advanced does NOT allow destroy frozen funds
    expect(rules.destroyFrozenFunds.authorizedActionTaker).toBe("noOne");
  });

  it("allAllowed enables all actions including destroy frozen funds", () => {
    const rules = applyPresetToControlRules("allAllowed");
    expect(rules.manualMinting.authorizedActionTaker).toBe("contractOwner");
    expect(rules.manualBurning.authorizedActionTaker).toBe("contractOwner");
    expect(rules.freeze.authorizedActionTaker).toBe("contractOwner");
    expect(rules.unfreeze.authorizedActionTaker).toBe("contractOwner");
    expect(rules.destroyFrozenFunds.authorizedActionTaker).toBe("contractOwner");
    expect(rules.emergencyAction.authorizedActionTaker).toBe("contractOwner");
    expect(rules.maxSupplyChange.authorizedActionTaker).toBe("contractOwner");
    expect(rules.conventionsChange.authorizedActionTaker).toBe("contractOwner");
    expect(rules.marketplace.authorizedActionTaker).toBe("contractOwner");
    expect(rules.directPurchasePricing.authorizedActionTaker).toBe("contractOwner");
    expect(rules.mainControlGroupChange).toBe("contractOwner");
  });

  it("mostRestrictive sets restrictive marketplace rules", () => {
    const rules = applyPresetToControlRules("mostRestrictive");
    expect(rules.marketplace.changingAuthorizedToNoOneAllowed).toBe(false);
    expect(rules.marketplace.changingAdminToNoOneAllowed).toBe(false);
    expect(rules.marketplace.selfChangingAdminAllowed).toBe(false);
  });
});

// ─── SimpleInfoForm component tests ─────────────────────────────────

describe("SimpleInfoForm", () => {
  let onChange: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onChange = vi.fn();
  });

  it("renders the simple info form", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("simple-info-form")).toBeInTheDocument();
  });

  it("renders token name input", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("simple-token-name")).toBeInTheDocument();
  });

  it("renders description input with character counter", () => {
    render(
      <SimpleInfoForm
        state={{ ...createDefaultBasicInfo(), description: "Hello" }}
        onChange={onChange}
      />,
    );
    expect(screen.getByTestId("simple-description")).toBeInTheDocument();
    expect(screen.getByText("5/100")).toBeInTheDocument();
  });

  it("renders base supply input", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("simple-base-supply")).toBeInTheDocument();
  });

  it("renders max supply input", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("simple-max-supply")).toBeInTheDocument();
  });

  it("renders preset selector", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.getByTestId("simple-preset-select")).toBeInTheDocument();
  });

  it("shows validation error when no preset is selected", () => {
    const state = validBasicInfo({ preset: null });
    render(<SimpleInfoForm state={state} onChange={onChange} />);
    expect(screen.getByText("A token preset is required")).toBeInTheDocument();
  });

  it("does not show preset error when preset is selected", () => {
    const state = validBasicInfo({ preset: "allAllowed" });
    render(<SimpleInfoForm state={state} onChange={onChange} />);
    expect(screen.queryByText("A token preset is required")).not.toBeInTheDocument();
  });

  it("calls onChange when typing in token name", async () => {
    const user = userEvent.setup();
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    await user.type(screen.getByTestId("simple-token-name"), "A");
    expect(onChange).toHaveBeenCalled();
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0];
    expect(lastCall.names[0].singular).toBe("A");
  });

  it("calls onChange when typing in base supply (strips non-numeric)", async () => {
    const user = userEvent.setup();
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    await user.type(screen.getByTestId("simple-base-supply"), "abc123");
    const calls = onChange.mock.calls.map((c: BasicInfoState[]) => c[0].baseSupply);
    expect(calls.every((v: string) => /^[\d.]*$/.test(v))).toBe(true);
  });

  it("does not show multi-language controls", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.queryByTestId("add-language")).not.toBeInTheDocument();
    expect(screen.queryByTestId("language-select-0")).not.toBeInTheDocument();
  });

  it("does not show advanced options toggle", () => {
    render(<SimpleInfoForm state={createDefaultBasicInfo()} onChange={onChange} />);
    expect(screen.queryByTestId("advanced-toggle")).not.toBeInTheDocument();
  });
});

// ─── TokenCreatorWizard component tests — Simple Mode ───────────────

describe("TokenCreatorWizard — Simple Mode", () => {
  let onCancel: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onCancel = vi.fn();
  });

  it("renders the wizard container", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("token-creator-wizard")).toBeInTheDocument();
  });

  it("starts in Simple Mode by default", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("simple-info-form")).toBeInTheDocument();
    expect(screen.getByText("Simple Mode")).toBeInTheDocument();
  });

  it("shows 'Advanced Mode' toggle button", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("mode-toggle")).toHaveTextContent("Advanced Mode");
  });

  it("shows Cancel button", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("wizard-back")).toHaveTextContent("Cancel");
  });

  it("calls onCancel when Cancel is clicked", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);
    await user.click(screen.getByTestId("wizard-back"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("does not show step indicators in Simple Mode", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.queryByText("Step 1 of 7")).not.toBeInTheDocument();
    expect(screen.queryByTestId("wizard-next")).not.toBeInTheDocument();
  });

  it("shows review step inline", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("review-step")).toBeInTheDocument();
  });

  it("switches to Advanced Mode when toggle is clicked", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);
    await user.click(screen.getByTestId("mode-toggle"));
    expect(screen.getByTestId("basic-info-step")).toBeInTheDocument();
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("mode-toggle")).toHaveTextContent("Simple Mode");
  });
});

// ─── TokenCreatorWizard component tests — Advanced Mode ─────────────

describe("TokenCreatorWizard — Advanced Mode", () => {
  let onCancel: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onCancel = vi.fn();
  });

  async function switchToAdvanced() {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);
    await user.click(screen.getByTestId("mode-toggle"));
    return user;
  }

  it("renders all 7 step labels in the step indicator", async () => {
    await switchToAdvanced();
    for (const step of WIZARD_STEPS) {
      expect(screen.getByText(step.label)).toBeInTheDocument();
    }
  });

  it("starts at step 1 (Basic Info)", async () => {
    await switchToAdvanced();
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("basic-info-step")).toBeInTheDocument();
  });

  it("shows Cancel button on step 1", async () => {
    await switchToAdvanced();
    expect(screen.getByTestId("wizard-back")).toHaveTextContent("Cancel");
  });

  it("calls onCancel when Cancel is clicked on step 1", async () => {
    const user = await switchToAdvanced();
    await user.click(screen.getByTestId("wizard-back"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("disables Next button when basic info is not valid", async () => {
    await switchToAdvanced();
    expect(screen.getByTestId("wizard-next")).toBeDisabled();
  });

  it("does not show Create Token button on step 1", async () => {
    await switchToAdvanced();
    expect(screen.queryByTestId("wizard-create")).not.toBeInTheDocument();
  });

  it("shows Previous button on step 2+", async () => {
    const user = await switchToAdvanced();

    // Fill in valid basic info to enable Next
    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("wizard-back")).toHaveTextContent("Previous");
  });

  it("navigates back when Previous is clicked", async () => {
    const user = await switchToAdvanced();

    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();

    await user.click(screen.getByTestId("wizard-back"));
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("basic-info-step")).toBeInTheDocument();
  });

  it("shows distribution step on step 2", async () => {
    const user = await switchToAdvanced();

    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(
      screen.getByTestId("distribution-step"),
    ).toBeInTheDocument();
  });

  it("allows clicking on completed step indicators to navigate back", async () => {
    const user = await switchToAdvanced();

    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();

    // Click on step 1 indicator label
    await user.click(screen.getByText("Basic Info"));
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
  });

  it("shows Review step on the last step (no Next button)", async () => {
    const user = await switchToAdvanced();

    // Fill valid basic info
    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    // Navigate through all steps
    for (let i = 0; i < WIZARD_STEPS.length - 1; i++) {
      await user.click(screen.getByTestId("wizard-next"));
    }

    expect(screen.getByText(`Step 7 of 7`)).toBeInTheDocument();
    expect(screen.getByTestId("review-step")).toBeInTheDocument();
    expect(screen.queryByTestId("wizard-next")).not.toBeInTheDocument();
  });

  it("preserves basic info state when navigating away and back", async () => {
    const user = await switchToAdvanced();

    await user.type(screen.getByTestId("token-name-singular-0"), "MyToken");
    await user.type(screen.getByTestId("base-supply"), "5000");

    // Navigate forward
    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();

    // Navigate back
    await user.click(screen.getByTestId("wizard-back"));

    // State should be preserved
    expect(screen.getByTestId("token-name-singular-0")).toHaveValue("MyToken");
    expect(screen.getByTestId("base-supply")).toHaveValue("5000");
  });

  it("shows Simple Mode toggle button", async () => {
    await switchToAdvanced();
    expect(screen.getByTestId("mode-toggle")).toHaveTextContent("Simple Mode");
  });

  it("switches back to Simple Mode when toggle is clicked", async () => {
    const user = await switchToAdvanced();
    await user.click(screen.getByTestId("mode-toggle"));
    expect(screen.getByTestId("simple-info-form")).toBeInTheDocument();
    expect(screen.getByTestId("mode-toggle")).toHaveTextContent("Advanced Mode");
  });

  it("preserves token name when switching between modes", async () => {
    const user = await switchToAdvanced();

    // Type in Advanced Mode
    await user.type(screen.getByTestId("token-name-singular-0"), "MyToken");

    // Switch to Simple Mode
    await user.click(screen.getByTestId("mode-toggle"));
    expect(screen.getByTestId("simple-token-name")).toHaveValue("MyToken");
  });
});
