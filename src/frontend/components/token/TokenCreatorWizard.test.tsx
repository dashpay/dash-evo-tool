import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  TokenCreatorWizard,
  BasicInfoStep,
  validateBasicInfo,
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

// ─── TokenCreatorWizard component tests ─────────────────────────────

describe("TokenCreatorWizard", () => {
  let onCancel: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onCancel = vi.fn();
  });

  it("renders the wizard container", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("token-creator-wizard")).toBeInTheDocument();
  });

  it("renders all 7 step labels in the step indicator", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    for (const step of WIZARD_STEPS) {
      expect(screen.getByText(step.label)).toBeInTheDocument();
    }
  });

  it("starts at step 1 (Basic Info)", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("basic-info-step")).toBeInTheDocument();
  });

  it("shows Cancel button on step 1", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("wizard-back")).toHaveTextContent("Cancel");
  });

  it("calls onCancel when Cancel is clicked on step 1", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);
    await user.click(screen.getByTestId("wizard-back"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("disables Next button when basic info is not valid", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.getByTestId("wizard-next")).toBeDisabled();
  });

  it("does not show Create Token button on step 1", () => {
    render(<TokenCreatorWizard onCancel={onCancel} />);
    expect(screen.queryByTestId("wizard-create")).not.toBeInTheDocument();
  });

  it("shows Previous button on step 2+", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);

    // Fill in valid basic info to enable Next
    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("wizard-back")).toHaveTextContent("Previous");
  });

  it("navigates back when Previous is clicked", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);

    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();

    await user.click(screen.getByTestId("wizard-back"));
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
    expect(screen.getByTestId("basic-info-step")).toBeInTheDocument();
  });

  it("shows placeholder for unimplemented steps (step 2)", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);

    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(
      screen.getByTestId("placeholder-step-distribution"),
    ).toBeInTheDocument();
    expect(screen.getByText("Distribution — coming soon")).toBeInTheDocument();
  });

  it("allows clicking on completed step indicators to navigate back", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);

    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    await user.click(screen.getByTestId("wizard-next"));
    expect(screen.getByText("Step 2 of 7")).toBeInTheDocument();

    // Click on step 1 indicator label
    await user.click(screen.getByText("Basic Info"));
    expect(screen.getByText("Step 1 of 7")).toBeInTheDocument();
  });

  it("shows Create Token button on the last step", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);

    // Fill valid basic info
    await user.type(screen.getByTestId("token-name-singular-0"), "TestToken");
    await user.type(screen.getByTestId("base-supply"), "1000");

    // Navigate through all steps
    for (let i = 0; i < WIZARD_STEPS.length - 1; i++) {
      await user.click(screen.getByTestId("wizard-next"));
    }

    expect(screen.getByText(`Step 7 of 7`)).toBeInTheDocument();
    expect(screen.getByTestId("wizard-create")).toBeInTheDocument();
    expect(screen.queryByTestId("wizard-next")).not.toBeInTheDocument();
  });

  it("preserves basic info state when navigating away and back", async () => {
    const user = userEvent.setup();
    render(<TokenCreatorWizard onCancel={onCancel} />);

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
});
