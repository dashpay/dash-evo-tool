import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { LoadingSpinner, LoadingOverlay } from "./LoadingSpinner";

describe("LoadingSpinner", () => {
  it("renders spinner icon", () => {
    const { container } = render(<LoadingSpinner />);
    const svg = container.querySelector("svg");
    expect(svg).toBeInTheDocument();
    expect(svg).toHaveClass("animate-spin");
  });

  it("has status role with default label", () => {
    render(<LoadingSpinner />);
    const el = screen.getByRole("status");
    expect(el).toHaveAttribute("aria-label", "Loading");
  });

  it("shows label text when provided", () => {
    render(<LoadingSpinner label="Syncing wallets..." />);
    expect(screen.getByText("Syncing wallets...")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveAttribute(
      "aria-label",
      "Syncing wallets...",
    );
  });

  it("does not show label text when omitted", () => {
    const { container } = render(<LoadingSpinner />);
    const spans = container.querySelectorAll("span");
    expect(spans).toHaveLength(0);
  });

  it("applies sm size", () => {
    const { container } = render(<LoadingSpinner size="sm" />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveClass("h-4", "w-4");
  });

  it("applies md size (default)", () => {
    const { container } = render(<LoadingSpinner />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveClass("h-6", "w-6");
  });

  it("applies lg size", () => {
    const { container } = render(<LoadingSpinner size="lg" />);
    const svg = container.querySelector("svg");
    expect(svg).toHaveClass("h-10", "w-10");
  });

  it("applies custom className", () => {
    const { container } = render(<LoadingSpinner className="my-custom" />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("my-custom");
  });
});

describe("LoadingOverlay", () => {
  it("renders nothing when not visible", () => {
    const { container } = render(<LoadingOverlay visible={false} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders overlay when visible", () => {
    render(<LoadingOverlay visible />);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("renders spinner icon when visible", () => {
    const { container } = render(<LoadingOverlay visible />);
    const svg = container.querySelector("svg");
    expect(svg).toBeInTheDocument();
    expect(svg).toHaveClass("animate-spin");
  });

  it("shows label when provided", () => {
    render(<LoadingOverlay visible label="Initializing app..." />);
    expect(screen.getByText("Initializing app...")).toBeInTheDocument();
  });

  it("has default aria-label when no label", () => {
    render(<LoadingOverlay visible />);
    expect(screen.getByRole("status")).toHaveAttribute("aria-label", "Loading");
  });

  it("uses provided label for aria-label", () => {
    render(<LoadingOverlay visible label="Please wait" />);
    expect(screen.getByRole("status")).toHaveAttribute(
      "aria-label",
      "Please wait",
    );
  });

  it("applies backdrop styling", () => {
    render(<LoadingOverlay visible />);
    const el = screen.getByRole("status");
    expect(el).toHaveClass("absolute", "inset-0", "z-50", "backdrop-blur-sm");
  });

  it("applies custom className", () => {
    render(<LoadingOverlay visible className="my-custom" />);
    const el = screen.getByRole("status");
    expect(el).toHaveClass("my-custom");
  });
});
