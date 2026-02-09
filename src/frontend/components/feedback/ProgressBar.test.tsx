import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { ProgressBar } from "./ProgressBar";

describe("ProgressBar", () => {
  it("renders progressbar role", () => {
    render(<ProgressBar value={50} />);
    expect(screen.getByRole("progressbar")).toBeInTheDocument();
  });

  it("sets aria-valuenow correctly", () => {
    render(<ProgressBar value={75} />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "75");
  });

  it("sets aria-valuemin and aria-valuemax", () => {
    render(<ProgressBar value={50} />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuemin", "0");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
  });

  it("clamps value at 0", () => {
    render(<ProgressBar value={-10} />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "0");
  });

  it("clamps value at 100", () => {
    render(<ProgressBar value={150} />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "100");
  });

  it("calculates percentage from custom max", () => {
    render(<ProgressBar value={250} max={500} />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "50");
  });

  it("renders fill bar with correct width", () => {
    const { container } = render(<ProgressBar value={40} />);
    const fill = container.querySelector("[role='progressbar'] > div");
    expect(fill).toHaveStyle({ width: "40%" });
  });

  it("renders label when provided", () => {
    render(<ProgressBar value={50} label="SPV Sync" />);
    expect(screen.getByText("SPV Sync")).toBeInTheDocument();
  });

  it("does not render label row when no label and no percent", () => {
    const { container } = render(<ProgressBar value={50} />);
    // Only the bar track and fill should exist
    expect(container.querySelector(".mb-1\\.5")).not.toBeInTheDocument();
  });

  it("shows percentage when showPercent is true", () => {
    render(<ProgressBar value={73} showPercent />);
    expect(screen.getByText("73%")).toBeInTheDocument();
  });

  it("uses label as aria-label", () => {
    render(<ProgressBar value={50} label="Downloading" />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-label", "Downloading");
  });

  it("uses fallback aria-label when no label", () => {
    render(<ProgressBar value={50} />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-label", "Progress");
  });

  it("applies primary variant by default", () => {
    const { container } = render(<ProgressBar value={50} />);
    const fill = container.querySelector("[role='progressbar'] > div");
    expect(fill).toHaveClass("bg-primary");
  });

  it("applies success variant", () => {
    const { container } = render(
      <ProgressBar value={50} variant="success" />,
    );
    const fill = container.querySelector("[role='progressbar'] > div");
    expect(fill).toHaveClass("bg-success");
  });

  it("applies destructive variant", () => {
    const { container } = render(
      <ProgressBar value={50} variant="destructive" />,
    );
    const fill = container.querySelector("[role='progressbar'] > div");
    expect(fill).toHaveClass("bg-destructive");
  });

  it("applies custom className", () => {
    const { container } = render(
      <ProgressBar value={50} className="my-custom" />,
    );
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("my-custom");
  });
});
