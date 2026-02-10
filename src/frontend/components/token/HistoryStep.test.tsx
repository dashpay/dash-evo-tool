import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  HistoryStep,
  createDefaultHistoryState,
  computeParentState,
} from "./HistoryStep";
import type { HistoryState } from "./HistoryStep";

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── createDefaultHistoryState ──────────────────────────────────────

describe("createDefaultHistoryState", () => {
  it("defaults all history flags to true", () => {
    const state = createDefaultHistoryState();
    expect(state.keepTransferHistory).toBe(true);
    expect(state.keepFreezingHistory).toBe(true);
    expect(state.keepMintingHistory).toBe(true);
    expect(state.keepBurningHistory).toBe(true);
    expect(state.keepDirectPricingHistory).toBe(true);
    expect(state.keepDirectPurchaseHistory).toBe(true);
  });

  it("defaults showAdvanced to false", () => {
    expect(createDefaultHistoryState().showAdvanced).toBe(false);
  });
});

// ─── computeParentState ─────────────────────────────────────────────

describe("computeParentState", () => {
  it("returns true when all flags are on", () => {
    expect(computeParentState(createDefaultHistoryState())).toBe(true);
  });

  it("returns false when all flags are off", () => {
    const state: HistoryState = {
      keepTransferHistory: false,
      keepFreezingHistory: false,
      keepMintingHistory: false,
      keepBurningHistory: false,
      keepDirectPricingHistory: false,
      keepDirectPurchaseHistory: false,
      showAdvanced: false,
    };
    expect(computeParentState(state)).toBe(false);
  });

  it('returns "indeterminate" when flags are mixed', () => {
    const state: HistoryState = {
      ...createDefaultHistoryState(),
      keepTransferHistory: false,
    };
    expect(computeParentState(state)).toBe("indeterminate");
  });
});

// ─── HistoryStep rendering ──────────────────────────────────────────

describe("HistoryStep", () => {
  let onChange: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onChange = vi.fn();
  });

  it("renders the step container", () => {
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    expect(screen.getByTestId("history-step")).toBeInTheDocument();
  });

  it("shows description text", () => {
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    expect(screen.getByText(/Configure which token operations/)).toBeInTheDocument();
  });

  it("renders parent checkbox", () => {
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    expect(screen.getByTestId("keep-history-parent")).toBeInTheDocument();
    expect(screen.getByText("Keep history")).toBeInTheDocument();
  });

  it("renders Advanced toggle", () => {
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    expect(screen.getByTestId("history-advanced-toggle")).toBeInTheDocument();
  });

  it("shows summary when advanced is not expanded", () => {
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    expect(screen.getByTestId("history-summary")).toBeInTheDocument();
    expect(screen.getByText("All history is being recorded.")).toBeInTheDocument();
  });

  it("shows 'no history' summary when all off", () => {
    const state: HistoryState = {
      keepTransferHistory: false,
      keepFreezingHistory: false,
      keepMintingHistory: false,
      keepBurningHistory: false,
      keepDirectPricingHistory: false,
      keepDirectPurchaseHistory: false,
      showAdvanced: false,
    };
    render(<HistoryStep state={state} onChange={onChange} />);
    expect(screen.getByText("No history is being recorded.")).toBeInTheDocument();
  });

  it("shows partial summary when mixed", () => {
    const state: HistoryState = {
      ...createDefaultHistoryState(),
      keepTransferHistory: false,
      keepBurningHistory: false,
    };
    render(<HistoryStep state={state} onChange={onChange} />);
    expect(screen.getByText(/4 of 6 operation types/)).toBeInTheDocument();
  });

  it("does not show sub-checkboxes by default", () => {
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    expect(screen.queryByTestId("history-advanced-section")).not.toBeInTheDocument();
  });

  it("shows sub-checkboxes when advanced is expanded", () => {
    const state = { ...createDefaultHistoryState(), showAdvanced: true };
    render(<HistoryStep state={state} onChange={onChange} />);
    expect(screen.getByTestId("history-advanced-section")).toBeInTheDocument();
    expect(screen.getByTestId("history-keepTransferHistory")).toBeInTheDocument();
    expect(screen.getByTestId("history-keepFreezingHistory")).toBeInTheDocument();
    expect(screen.getByTestId("history-keepMintingHistory")).toBeInTheDocument();
    expect(screen.getByTestId("history-keepBurningHistory")).toBeInTheDocument();
    expect(screen.getByTestId("history-keepDirectPricingHistory")).toBeInTheDocument();
    expect(screen.getByTestId("history-keepDirectPurchaseHistory")).toBeInTheDocument();
  });

  it("shows sub-checkbox labels", () => {
    const state = { ...createDefaultHistoryState(), showAdvanced: true };
    render(<HistoryStep state={state} onChange={onChange} />);
    expect(screen.getByText("Transfers")).toBeInTheDocument();
    expect(screen.getByText("Freezes / unfreezes")).toBeInTheDocument();
    expect(screen.getByText("Mints")).toBeInTheDocument();
    expect(screen.getByText("Burns")).toBeInTheDocument();
    expect(screen.getByText("Direct-pricing changes")).toBeInTheDocument();
    expect(screen.getByText("Direct purchases")).toBeInTheDocument();
  });

  it("expands advanced section when toggle is clicked", async () => {
    const user = userEvent.setup();
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    await user.click(screen.getByTestId("history-advanced-toggle"));
    const call = onChange.mock.calls[0][0] as HistoryState;
    expect(call.showAdvanced).toBe(true);
  });

  it("collapses advanced section when toggle is clicked again", async () => {
    const user = userEvent.setup();
    const state = { ...createDefaultHistoryState(), showAdvanced: true };
    render(<HistoryStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("history-advanced-toggle"));
    const call = onChange.mock.calls[0][0] as HistoryState;
    expect(call.showAdvanced).toBe(false);
  });

  it("toggles all flags to false when parent is clicked (all currently on)", async () => {
    const user = userEvent.setup();
    render(<HistoryStep state={createDefaultHistoryState()} onChange={onChange} />);
    await user.click(screen.getByTestId("keep-history-parent"));
    const call = onChange.mock.calls[0][0] as HistoryState;
    expect(call.keepTransferHistory).toBe(false);
    expect(call.keepFreezingHistory).toBe(false);
    expect(call.keepMintingHistory).toBe(false);
    expect(call.keepBurningHistory).toBe(false);
    expect(call.keepDirectPricingHistory).toBe(false);
    expect(call.keepDirectPurchaseHistory).toBe(false);
  });

  it("toggles all flags to true when parent is clicked (all currently off)", async () => {
    const user = userEvent.setup();
    const state: HistoryState = {
      keepTransferHistory: false,
      keepFreezingHistory: false,
      keepMintingHistory: false,
      keepBurningHistory: false,
      keepDirectPricingHistory: false,
      keepDirectPurchaseHistory: false,
      showAdvanced: false,
    };
    render(<HistoryStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("keep-history-parent"));
    const call = onChange.mock.calls[0][0] as HistoryState;
    expect(call.keepTransferHistory).toBe(true);
    expect(call.keepFreezingHistory).toBe(true);
    expect(call.keepMintingHistory).toBe(true);
    expect(call.keepBurningHistory).toBe(true);
    expect(call.keepDirectPricingHistory).toBe(true);
    expect(call.keepDirectPurchaseHistory).toBe(true);
  });

  it("toggles all to true when parent clicked in indeterminate state", async () => {
    const user = userEvent.setup();
    const state: HistoryState = {
      ...createDefaultHistoryState(),
      keepTransferHistory: false,
    };
    render(<HistoryStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("keep-history-parent"));
    const call = onChange.mock.calls[0][0] as HistoryState;
    expect(call.keepTransferHistory).toBe(true);
    expect(call.keepFreezingHistory).toBe(true);
  });

  it("toggles individual sub-checkbox", async () => {
    const user = userEvent.setup();
    const state = { ...createDefaultHistoryState(), showAdvanced: true };
    render(<HistoryStep state={state} onChange={onChange} />);
    await user.click(screen.getByTestId("history-keepTransferHistory"));
    const call = onChange.mock.calls[0][0] as HistoryState;
    expect(call.keepTransferHistory).toBe(false);
    // Others unchanged
    expect(call.keepFreezingHistory).toBe(true);
    expect(call.keepMintingHistory).toBe(true);
  });
});
