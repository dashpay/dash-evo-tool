import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  KeywordsStep,
  createDefaultKeywordsState,
  validateKeywords,
  validateSingleKeyword,
} from "./KeywordsStep";
import type { KeywordsState } from "./KeywordsStep";

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── createDefaultKeywordsState ─────────────────────────────────────

describe("createDefaultKeywordsState", () => {
  it("returns empty keywords array", () => {
    const state = createDefaultKeywordsState();
    expect(state.keywords).toHaveLength(0);
  });

  it("returns empty current input", () => {
    expect(createDefaultKeywordsState().currentInput).toBe("");
  });
});

// ─── validateKeywords ───────────────────────────────────────────────

describe("validateKeywords", () => {
  it("validates empty keywords as valid", () => {
    const result = validateKeywords(createDefaultKeywordsState());
    expect(result.valid).toBe(true);
  });

  it("validates well-formed keywords", () => {
    const state: KeywordsState = {
      keywords: ["gaming", "reward", "loyalty"],
      currentInput: "",
    };
    const result = validateKeywords(state);
    expect(result.valid).toBe(true);
  });

  it("rejects keyword shorter than 3 chars", () => {
    const state: KeywordsState = { keywords: ["ab"], currentInput: "" };
    const result = validateKeywords(state);
    expect(result.valid).toBe(false);
    expect(result.errors["keyword_0"]).toContain("at least 3");
  });

  it("rejects keyword longer than 50 chars", () => {
    const state: KeywordsState = { keywords: ["a".repeat(51)], currentInput: "" };
    const result = validateKeywords(state);
    expect(result.valid).toBe(false);
    expect(result.errors["keyword_0"]).toContain("at most 50");
  });

  it("detects duplicate keywords (case-insensitive)", () => {
    const state: KeywordsState = { keywords: ["gaming", "Gaming"], currentInput: "" };
    const result = validateKeywords(state);
    expect(result.errors["keyword_1_dup"]).toContain("Duplicate");
  });
});

// ─── validateSingleKeyword ──────────────────────────────────────────

describe("validateSingleKeyword", () => {
  it("rejects empty keyword", () => {
    expect(validateSingleKeyword("", [])).toBe("Keyword cannot be empty");
  });

  it("rejects short keyword", () => {
    expect(validateSingleKeyword("ab", [])).toContain("at least 3");
  });

  it("rejects long keyword", () => {
    expect(validateSingleKeyword("a".repeat(51), [])).toContain("at most 50");
  });

  it("detects duplicate", () => {
    expect(validateSingleKeyword("gaming", ["Gaming"])).toContain("Duplicate");
  });

  it("accepts valid keyword", () => {
    expect(validateSingleKeyword("gaming", [])).toBeNull();
  });
});

// ─── KeywordsStep rendering ─────────────────────────────────────────

describe("KeywordsStep", () => {
  let onChange: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onChange = vi.fn();
  });

  it("renders the step container", () => {
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    expect(screen.getByTestId("keywords-step")).toBeInTheDocument();
  });

  it("shows description text", () => {
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    expect(screen.getByText(/Add searchable keywords/)).toBeInTheDocument();
  });

  it("shows keyword input", () => {
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    expect(screen.getByTestId("keyword-input")).toBeInTheDocument();
  });

  it("shows Add button", () => {
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    expect(screen.getByTestId("add-keyword")).toBeInTheDocument();
  });

  it("disables Add button when input is empty", () => {
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    expect(screen.getByTestId("add-keyword")).toBeDisabled();
  });

  it("shows empty state when no keywords", () => {
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    expect(screen.getByTestId("keywords-empty-state")).toBeInTheDocument();
    expect(screen.getByText(/No keywords added/)).toBeInTheDocument();
  });

  it("adds keyword when Add is clicked", async () => {
    const user = userEvent.setup();
    const state: KeywordsState = { keywords: [], currentInput: "gaming" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("add-keyword"));
    const call = onChange.mock.calls[0][0] as KeywordsState;
    expect(call.keywords).toContain("gaming");
    expect(call.currentInput).toBe("");
  });

  it("adds keyword on Enter key", async () => {
    const user = userEvent.setup();
    const state: KeywordsState = { keywords: [], currentInput: "reward" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    await user.type(screen.getByTestId("keyword-input"), "{Enter}");
    const call = onChange.mock.calls[onChange.mock.calls.length - 1][0] as KeywordsState;
    expect(call.keywords).toContain("reward");
  });

  it("adds multiple comma-separated keywords at once", async () => {
    const user = userEvent.setup();
    const state: KeywordsState = { keywords: [], currentInput: "gaming, reward, loyalty" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("add-keyword"));
    const call = onChange.mock.calls[0][0] as KeywordsState;
    expect(call.keywords).toEqual(["gaming", "reward", "loyalty"]);
  });

  it("renders keyword badges", () => {
    const state: KeywordsState = { keywords: ["gaming", "reward"], currentInput: "" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    expect(screen.getByTestId("keywords-list")).toBeInTheDocument();
    expect(screen.getByTestId("keyword-badge-0")).toBeInTheDocument();
    expect(screen.getByTestId("keyword-badge-1")).toBeInTheDocument();
    expect(screen.getByText("gaming")).toBeInTheDocument();
    expect(screen.getByText("reward")).toBeInTheDocument();
  });

  it("shows keyword count", () => {
    const state: KeywordsState = { keywords: ["gaming", "reward"], currentInput: "" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    expect(screen.getByText("Keywords (2)")).toBeInTheDocument();
  });

  it("shows cost estimate", () => {
    const state: KeywordsState = { keywords: ["gaming", "reward"], currentInput: "" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    expect(screen.getByTestId("keywords-cost")).toBeInTheDocument();
    expect(screen.getByText("0.2 Dash")).toBeInTheDocument();
  });

  it("removes keyword when X is clicked", async () => {
    const user = userEvent.setup();
    const state: KeywordsState = { keywords: ["gaming", "reward"], currentInput: "" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("keyword-remove-0"));
    const call = onChange.mock.calls[0][0] as KeywordsState;
    expect(call.keywords).toEqual(["reward"]);
  });

  it("shows input error for short keyword in real-time", () => {
    const state: KeywordsState = { keywords: [], currentInput: "ab" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    expect(screen.getByTestId("keyword-input-error")).toBeInTheDocument();
    expect(screen.getByText(/at least 3/)).toBeInTheDocument();
  });

  it("shows duplicate error in input", () => {
    const state: KeywordsState = { keywords: ["gaming"], currentInput: "gaming" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    expect(screen.getByTestId("keyword-input-error")).toBeInTheDocument();
    expect(screen.getByText(/Duplicate/)).toBeInTheDocument();
  });

  it("updates input value on typing", async () => {
    const user = userEvent.setup();
    render(<KeywordsStep state={createDefaultKeywordsState()} onChange={onChange} />);
    await user.type(screen.getByTestId("keyword-input"), "test");
    expect(onChange).toHaveBeenCalled();
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0] as KeywordsState;
    expect(lastCall.currentInput).toBe("t");
  });

  it("hides empty state when keywords exist", () => {
    const state: KeywordsState = { keywords: ["gaming"], currentInput: "" };
    render(<KeywordsStep state={state} onChange={onChange} />);
    expect(screen.queryByTestId("keywords-empty-state")).not.toBeInTheDocument();
  });
});
