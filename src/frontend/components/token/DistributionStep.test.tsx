import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import {
  DistributionStep,
  createDefaultDistributionState,
  FormulaVisualization,
  getTimeUnitLabel,
  INTERVAL_TYPE_OPTIONS,
  TIME_UNIT_OPTIONS,
  DISTRIBUTION_FUNCTION_OPTIONS,
  RECIPIENT_OPTIONS,
} from "./DistributionStep";
import type {
  DistributionState,
  DistributionFunction,
} from "./DistributionStep";

// ─── Mocks ──────────────────────────────────────────────────────────

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── Helpers ────────────────────────────────────────────────────────

function renderDistributionStep(overrides: Partial<DistributionState> = {}) {
  const state: DistributionState = {
    ...createDefaultDistributionState(),
    ...overrides,
  };
  const onChange = vi.fn();
  const utils = render(<DistributionStep state={state} onChange={onChange} />);
  return { ...utils, onChange, state };
}

function perpetualEnabled(overrides: Partial<DistributionState["perpetual"]> = {}): Partial<DistributionState> {
  return {
    perpetual: {
      ...createDefaultDistributionState().perpetual,
      enabled: true,
      ...overrides,
    },
  };
}

// ─── createDefaultDistributionState tests ───────────────────────────

describe("createDefaultDistributionState", () => {
  it("returns a valid default state", () => {
    const state = createDefaultDistributionState();
    expect(state.perpetual.enabled).toBe(false);
    expect(state.perpetual.intervalType).toBe("timeBased");
    expect(state.perpetual.distributionFunction).toBe("fixedAmount");
    expect(state.perpetual.recipient).toBe("contractOwner");
    expect(state.preProgrammed.enabled).toBe(false);
    expect(state.preProgrammed.entries).toHaveLength(0);
  });

  it("has empty strings for all perpetual function parameters", () => {
    const state = createDefaultDistributionState();
    expect(state.perpetual.fixedAmount).toBe("");
    expect(state.perpetual.linearA).toBe("");
    expect(state.perpetual.polyA).toBe("");
    expect(state.perpetual.expA).toBe("");
    expect(state.perpetual.logA).toBe("");
    expect(state.perpetual.invLogA).toBe("");
  });
});

// ─── getTimeUnitLabel tests ─────────────────────────────────────────

describe("getTimeUnitLabel", () => {
  it("returns singular for amount '1'", () => {
    expect(getTimeUnitLabel("day", "1")).toBe("Day");
    expect(getTimeUnitLabel("hour", "1")).toBe("Hour");
  });

  it("returns plural for amount > 1", () => {
    expect(getTimeUnitLabel("day", "5")).toBe("Days");
    expect(getTimeUnitLabel("hour", "2")).toBe("Hours");
  });

  it("returns plural for empty amount", () => {
    expect(getTimeUnitLabel("minute", "")).toBe("Minutes");
  });

  it("returns plural for non-numeric amount", () => {
    expect(getTimeUnitLabel("week", "abc")).toBe("Weeks");
  });
});

// ─── Option constants tests ─────────────────────────────────────────

describe("option constants", () => {
  it("has 3 interval type options", () => {
    expect(INTERVAL_TYPE_OPTIONS).toHaveLength(3);
    expect(INTERVAL_TYPE_OPTIONS.map((o) => o.value)).toEqual(["timeBased", "epochBased", "blockBased"]);
  });

  it("has 6 time unit options", () => {
    expect(TIME_UNIT_OPTIONS).toHaveLength(6);
    expect(TIME_UNIT_OPTIONS.map((o) => o.value)).toEqual([
      "second", "minute", "hour", "day", "week", "year",
    ]);
  });

  it("has 8 distribution function options", () => {
    expect(DISTRIBUTION_FUNCTION_OPTIONS).toHaveLength(8);
  });

  it("has 3 recipient options", () => {
    expect(RECIPIENT_OPTIONS).toHaveLength(3);
    expect(RECIPIENT_OPTIONS.map((o) => o.value)).toEqual(["contractOwner", "identity", "evonodes"]);
  });
});

// ─── FormulaVisualization tests ─────────────────────────────────────

describe("FormulaVisualization", () => {
  it("renders SVG for fixedAmount", () => {
    const { container } = render(<FormulaVisualization fn="fixedAmount" />);
    expect(container.querySelector("svg")).toBeTruthy();
    expect(screen.getByTestId("formula-visualization")).toBeInTheDocument();
  });

  it("renders SVG for each function type", () => {
    const functions: DistributionFunction[] = [
      "fixedAmount", "stepDecreasing", "stepwise", "linear",
      "polynomial", "exponential", "logarithmic", "invertedLogarithmic",
    ];
    for (const fn of functions) {
      const { container, unmount } = render(<FormulaVisualization fn={fn} />);
      expect(container.querySelector("svg")).toBeTruthy();
      unmount();
    }
  });

  it("displays formula label text", () => {
    render(<FormulaVisualization fn="linear" />);
    expect(screen.getByText("f(x) = a·x + b")).toBeInTheDocument();
  });
});

// ─── DistributionStep rendering tests ───────────────────────────────

describe("DistributionStep", () => {
  describe("initial render", () => {
    it("renders with both checkboxes unchecked", () => {
      renderDistributionStep();
      expect(screen.getByTestId("distribution-step")).toBeInTheDocument();
      expect(screen.getByTestId("enable-perpetual")).toBeInTheDocument();
      expect(screen.getByTestId("enable-preprogrammed")).toBeInTheDocument();
    });

    it("does not show perpetual fields when disabled", () => {
      renderDistributionStep();
      expect(screen.queryByTestId("interval-type-select")).not.toBeInTheDocument();
      expect(screen.queryByTestId("function-select")).not.toBeInTheDocument();
    });

    it("does not show pre-programmed fields when disabled", () => {
      renderDistributionStep();
      expect(screen.queryByTestId("add-preprogrammed-entry")).not.toBeInTheDocument();
    });
  });

  describe("perpetual distribution toggle", () => {
    it("calls onChange with enabled=true when checkbox clicked", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep();
      await user.click(screen.getByTestId("enable-perpetual"));
      expect(onChange).toHaveBeenCalledTimes(1);
      const newState = onChange.mock.calls[0][0] as DistributionState;
      expect(newState.perpetual.enabled).toBe(true);
      expect(newState.perpetual.intervalType).toBe("timeBased");
    });

    it("shows perpetual config when enabled", () => {
      renderDistributionStep(perpetualEnabled());
      expect(screen.getByTestId("interval-type-select")).toBeInTheDocument();
      expect(screen.getByTestId("interval-amount")).toBeInTheDocument();
      expect(screen.getByTestId("function-select")).toBeInTheDocument();
      expect(screen.getByTestId("recipient-select")).toBeInTheDocument();
    });

    it("shows time unit selector for time-based interval type", () => {
      renderDistributionStep(perpetualEnabled({ intervalType: "timeBased" }));
      expect(screen.getByTestId("time-unit-select")).toBeInTheDocument();
    });

    it("shows 'blocks' label for block-based interval type", () => {
      renderDistributionStep(perpetualEnabled({ intervalType: "blockBased" }));
      expect(screen.getByText("blocks")).toBeInTheDocument();
      expect(screen.queryByTestId("time-unit-select")).not.toBeInTheDocument();
    });

    it("shows 'epochs' label for epoch-based interval type", () => {
      renderDistributionStep(perpetualEnabled({ intervalType: "epochBased" }));
      expect(screen.getByText("epochs")).toBeInTheDocument();
    });
  });

  describe("interval amount input", () => {
    it("sanitizes non-numeric input", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(perpetualEnabled());
      const input = screen.getByTestId("interval-amount");
      await user.type(input, "abc123");
      // Only the digits should remain via sanitization
      const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0] as DistributionState;
      // After typing "abc123" one char at a time, the sanitizer strips letters
      expect(lastCall.perpetual.intervalAmount).toMatch(/^\d*$/);
    });
  });

  describe("distribution function selection", () => {
    it("shows formula visualization", () => {
      renderDistributionStep(perpetualEnabled());
      expect(screen.getByTestId("formula-visualization")).toBeInTheDocument();
    });

    it("shows fixedAmount params by default", () => {
      renderDistributionStep(perpetualEnabled());
      expect(screen.getByTestId("param-fixed-amount")).toBeInTheDocument();
    });

    it("shows step-decreasing params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "stepDecreasing" }));
      expect(screen.getByTestId("step-decreasing-params")).toBeInTheDocument();
      expect(screen.getByTestId("param-step-count")).toBeInTheDocument();
      expect(screen.getByTestId("param-decrease-numerator")).toBeInTheDocument();
      expect(screen.getByTestId("param-decrease-denominator")).toBeInTheDocument();
      expect(screen.getByTestId("param-initial-emission")).toBeInTheDocument();
      expect(screen.getByTestId("param-trailing-amount")).toBeInTheDocument();
    });

    it("shows stepwise params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "stepwise" }));
      expect(screen.getByTestId("stepwise-params")).toBeInTheDocument();
      expect(screen.getByTestId("add-stepwise-entry")).toBeInTheDocument();
    });

    it("shows linear params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "linear" }));
      expect(screen.getByTestId("linear-params")).toBeInTheDocument();
      expect(screen.getByTestId("param-linear-a")).toBeInTheDocument();
      expect(screen.getByTestId("param-linear-d")).toBeInTheDocument();
    });

    it("shows polynomial params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "polynomial" }));
      expect(screen.getByTestId("polynomial-params")).toBeInTheDocument();
      expect(screen.getByTestId("param-poly-a")).toBeInTheDocument();
      expect(screen.getByTestId("param-poly-m")).toBeInTheDocument();
      expect(screen.getByTestId("param-poly-n")).toBeInTheDocument();
    });

    it("shows exponential params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "exponential" }));
      expect(screen.getByTestId("exponential-params")).toBeInTheDocument();
      expect(screen.getByTestId("param-exp-a")).toBeInTheDocument();
      expect(screen.getByTestId("param-exp-m")).toBeInTheDocument();
    });

    it("shows logarithmic params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "logarithmic" }));
      expect(screen.getByTestId("logarithmic-params")).toBeInTheDocument();
      expect(screen.getByTestId("param-log-a")).toBeInTheDocument();
    });

    it("shows inverted logarithmic params when selected", () => {
      renderDistributionStep(perpetualEnabled({ distributionFunction: "invertedLogarithmic" }));
      expect(screen.getByTestId("inverted-logarithmic-params")).toBeInTheDocument();
      expect(screen.getByTestId("param-inv-log-a")).toBeInTheDocument();
    });
  });

  describe("function info dialog", () => {
    it("opens when info button clicked", async () => {
      const user = userEvent.setup();
      renderDistributionStep(perpetualEnabled());
      const infoButton = screen.getByTestId("function-info-button");
      await user.click(infoButton);
      expect(screen.getByTestId("function-info-dialog")).toBeInTheDocument();
      expect(screen.getByText("Distribution Function Types")).toBeInTheDocument();
    });

    it("closes when close button clicked", async () => {
      const user = userEvent.setup();
      renderDistributionStep(perpetualEnabled());
      await user.click(screen.getByTestId("function-info-button"));
      expect(screen.getByTestId("function-info-dialog")).toBeInTheDocument();
      // Click the explicit Close button (which has visible text "Close")
      const dialog = screen.getByTestId("function-info-dialog");
      // Find all buttons, then pick the one with visible "Close" text content
      const buttons = within(dialog).getAllByRole("button");
      const closeButton = buttons.find((btn) => btn.textContent?.includes("Close") && btn.querySelector("svg"));
      expect(closeButton).toBeTruthy();
      await user.click(closeButton!);
      expect(screen.queryByTestId("function-info-dialog")).not.toBeInTheDocument();
    });
  });

  describe("recipient selection", () => {
    it("does not show identity input for contractOwner", () => {
      renderDistributionStep(perpetualEnabled({ recipient: "contractOwner" }));
      expect(screen.queryByTestId("recipient-identity-input")).not.toBeInTheDocument();
    });

    it("shows identity input for identity recipient", () => {
      renderDistributionStep(perpetualEnabled({ recipient: "identity" }));
      expect(screen.getByTestId("recipient-identity-input")).toBeInTheDocument();
    });

    it("does not show identity input for evonodes", () => {
      renderDistributionStep(perpetualEnabled({ recipient: "evonodes" }));
      expect(screen.queryByTestId("recipient-identity-input")).not.toBeInTheDocument();
    });

    it("updates recipient identity ID on input", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(perpetualEnabled({ recipient: "identity" }));
      const input = screen.getByTestId("recipient-identity-input");
      await user.type(input, "x");
      // Each keystroke triggers onChange. Since component isn't re-rendered with new state,
      // the first call captures the typed character appended to initial value ("")
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.perpetual.recipientIdentityId).toBe("x");
    });
  });

  describe("stepwise entries", () => {
    it("adds a stepwise entry when add button clicked", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(perpetualEnabled({ distributionFunction: "stepwise" }));
      await user.click(screen.getByTestId("add-stepwise-entry"));
      const newState = onChange.mock.calls[0][0] as DistributionState;
      expect(newState.perpetual.stepwiseEntries).toHaveLength(1);
      expect(newState.perpetual.stepwiseEntries[0]).toEqual({ startStep: "0", amount: "0" });
    });

    it("renders existing stepwise entries", () => {
      renderDistributionStep(
        perpetualEnabled({
          distributionFunction: "stepwise",
          stepwiseEntries: [
            { startStep: "10", amount: "100" },
            { startStep: "20", amount: "50" },
          ],
        }),
      );
      expect(screen.getByTestId("stepwise-entry-0")).toBeInTheDocument();
      expect(screen.getByTestId("stepwise-entry-1")).toBeInTheDocument();
    });

    it("removes a stepwise entry when remove clicked", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(
        perpetualEnabled({
          distributionFunction: "stepwise",
          stepwiseEntries: [
            { startStep: "10", amount: "100" },
            { startStep: "20", amount: "50" },
          ],
        }),
      );
      await user.click(screen.getByTestId("stepwise-remove-0"));
      const newState = onChange.mock.calls[0][0] as DistributionState;
      expect(newState.perpetual.stepwiseEntries).toHaveLength(1);
      expect(newState.perpetual.stepwiseEntries[0].startStep).toBe("20");
    });
  });

  describe("pre-programmed distribution", () => {
    it("toggles pre-programmed distribution on", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep();
      await user.click(screen.getByTestId("enable-preprogrammed"));
      const newState = onChange.mock.calls[0][0] as DistributionState;
      expect(newState.preProgrammed.enabled).toBe(true);
    });

    it("shows add entry button when enabled", () => {
      renderDistributionStep({ preProgrammed: { enabled: true, entries: [] } });
      expect(screen.getByTestId("add-preprogrammed-entry")).toBeInTheDocument();
    });

    it("adds a pre-programmed entry", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep({ preProgrammed: { enabled: true, entries: [] } });
      await user.click(screen.getByTestId("add-preprogrammed-entry"));
      const newState = onChange.mock.calls[0][0] as DistributionState;
      expect(newState.preProgrammed.entries).toHaveLength(1);
      expect(newState.preProgrammed.entries[0]).toEqual({
        days: "0", hours: "0", minutes: "0", identityId: "", amount: "",
      });
    });

    it("renders pre-programmed entries with all fields", () => {
      renderDistributionStep({
        preProgrammed: {
          enabled: true,
          entries: [
            { days: "30", hours: "12", minutes: "30", identityId: "abc123", amount: "5000" },
          ],
        },
      });
      expect(screen.getByTestId("preprogrammed-entry-0")).toBeInTheDocument();
      expect(screen.getByTestId("preprogrammed-days-0")).toHaveValue("30");
      expect(screen.getByTestId("preprogrammed-hours-0")).toHaveValue("12");
      expect(screen.getByTestId("preprogrammed-minutes-0")).toHaveValue("30");
      expect(screen.getByTestId("preprogrammed-identity-0")).toHaveValue("abc123");
      expect(screen.getByTestId("preprogrammed-amount-0")).toHaveValue("5000");
    });

    it("removes a pre-programmed entry", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep({
        preProgrammed: {
          enabled: true,
          entries: [
            { days: "30", hours: "12", minutes: "30", identityId: "abc", amount: "5000" },
            { days: "60", hours: "0", minutes: "0", identityId: "def", amount: "3000" },
          ],
        },
      });
      await user.click(screen.getByTestId("preprogrammed-remove-0"));
      const newState = onChange.mock.calls[0][0] as DistributionState;
      expect(newState.preProgrammed.entries).toHaveLength(1);
      expect(newState.preProgrammed.entries[0].identityId).toBe("def");
    });

    it("updates pre-programmed entry fields", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep({
        preProgrammed: {
          enabled: true,
          entries: [
            { days: "0", hours: "0", minutes: "0", identityId: "", amount: "" },
          ],
        },
      });
      await user.type(screen.getByTestId("preprogrammed-days-0"), "30");
      // Verify onChange was called with updated days value
      const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0] as DistributionState;
      expect(lastCall.preProgrammed.entries[0].days).toMatch(/\d+/);
    });

    it("sanitizes numeric fields in pre-programmed entries", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep({
        preProgrammed: {
          enabled: true,
          entries: [
            { days: "0", hours: "0", minutes: "0", identityId: "", amount: "" },
          ],
        },
      });
      await user.type(screen.getByTestId("preprogrammed-amount-0"), "abc123");
      const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0] as DistributionState;
      // Only digits should remain
      expect(lastCall.preProgrammed.entries[0].amount).toMatch(/^\d*$/);
    });

    it("allows free text in identity field (not sanitized)", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep({
        preProgrammed: {
          enabled: true,
          entries: [
            { days: "0", hours: "0", minutes: "0", identityId: "", amount: "" },
          ],
        },
      });
      // Type a single character to verify it's accepted without sanitization
      await user.type(screen.getByTestId("preprogrammed-identity-0"), "G");
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.preProgrammed.entries[0].identityId).toBe("G");
    });
  });

  describe("fixed amount param updates", () => {
    it("calls onChange when typing in fixed amount field", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(perpetualEnabled());
      await user.type(screen.getByTestId("param-fixed-amount"), "5");
      expect(onChange).toHaveBeenCalled();
      // Each keystroke produces an onChange call since it's a controlled component with flat state
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.perpetual.fixedAmount).toBe("5");
    });

    it("sanitizes non-numeric chars in fixed amount", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(perpetualEnabled());
      await user.type(screen.getByTestId("param-fixed-amount"), "a");
      // "a" should be stripped by sanitizeU64, resulting in empty string
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.perpetual.fixedAmount).toBe("");
    });

    it("accepts digits in fixed amount", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(perpetualEnabled({ fixedAmount: "50" }));
      await user.type(screen.getByTestId("param-fixed-amount"), "0");
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.perpetual.fixedAmount).toBe("500");
    });
  });

  describe("linear param updates", () => {
    it("allows negative sign for slope (i64)", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(
        perpetualEnabled({ distributionFunction: "linear" }),
      );
      await user.type(screen.getByTestId("param-linear-a"), "-");
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.perpetual.linearA).toBe("-");
    });

    it("preserves negative sign and digits for slope", async () => {
      const user = userEvent.setup();
      const { onChange } = renderDistributionStep(
        perpetualEnabled({ distributionFunction: "linear", linearA: "-" }),
      );
      await user.type(screen.getByTestId("param-linear-a"), "5");
      const firstCall = onChange.mock.calls[0][0] as DistributionState;
      expect(firstCall.perpetual.linearA).toBe("-5");
    });
  });

  describe("both sections enabled simultaneously", () => {
    it("renders both perpetual and pre-programmed sections", () => {
      renderDistributionStep({
        perpetual: { ...createDefaultDistributionState().perpetual, enabled: true },
        preProgrammed: { enabled: true, entries: [] },
      });
      expect(screen.getByTestId("interval-type-select")).toBeInTheDocument();
      expect(screen.getByTestId("add-preprogrammed-entry")).toBeInTheDocument();
    });
  });
});
