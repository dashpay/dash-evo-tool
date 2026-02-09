import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FeeConfirmationDialog } from "./FeeConfirmationDialog";

describe("FeeConfirmationDialog", () => {
  const defaultProps = {
    open: true,
    onOpenChange: vi.fn(),
    estimatedFee: 226,
    requiredFee: 1000,
  };

  it("renders title and description", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(
      screen.getByText("Fee Confirmation Required"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "The network requires a higher fee than estimated.",
      ),
    ).toBeInTheDocument();
  });

  it("does not render when closed", () => {
    render(<FeeConfirmationDialog {...defaultProps} open={false} />);
    expect(
      screen.queryByText("Fee Confirmation Required"),
    ).not.toBeInTheDocument();
  });

  it("displays estimated fee label", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(screen.getByText("Estimated fee")).toBeInTheDocument();
  });

  it("displays required fee label", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(screen.getByText("Required fee")).toBeInTheDocument();
  });

  it("displays additional cost label", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(screen.getByText("Additional cost")).toBeInTheDocument();
  });

  it("displays DASH conversion for estimated fee", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    // 226 duffs = 0.00000226 DASH
    expect(screen.getByText(/0\.00000226/)).toBeInTheDocument();
  });

  it("displays DASH conversion for required fee", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    // 1000 duffs = 0.00001000 DASH
    expect(screen.getByText(/0\.00001000/)).toBeInTheDocument();
  });

  it("uses default unit of duffs", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    // Dialog renders in a portal, so query the body
    expect(document.body.textContent).toContain("duffs");
  });

  it("uses custom unit", () => {
    render(<FeeConfirmationDialog {...defaultProps} unit="credits" />);
    expect(document.body.textContent).toContain("credits");
  });

  it("renders Confirm & Send and Cancel buttons", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(screen.getByRole("button", { name: /Confirm/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("calls onResult with confirmed status and override fee", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(
      <FeeConfirmationDialog {...defaultProps} onResult={onResult} />,
    );
    await user.click(screen.getByRole("button", { name: /Confirm/i }));
    expect(onResult).toHaveBeenCalledWith({
      status: "confirmed",
      overrideFee: 1000,
    });
  });

  it("calls onResult with canceled status", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(
      <FeeConfirmationDialog {...defaultProps} onResult={onResult} />,
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onResult).toHaveBeenCalledWith({ status: "canceled" });
  });

  it("shows prompt text", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(
      screen.getByText("Would you like to proceed with the higher fee?"),
    ).toBeInTheDocument();
  });

  it("has accessible dialog role", () => {
    render(<FeeConfirmationDialog {...defaultProps} />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
