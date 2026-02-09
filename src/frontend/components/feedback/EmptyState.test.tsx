import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { Wallet } from "lucide-react";
import { EmptyState } from "./EmptyState";

describe("EmptyState", () => {
  it("renders title", () => {
    render(<EmptyState title="No wallets yet" />);
    expect(screen.getByText("No wallets yet")).toBeInTheDocument();
  });

  it("renders description when provided", () => {
    render(
      <EmptyState
        title="No wallets"
        description="Create a wallet to get started"
      />,
    );
    expect(
      screen.getByText("Create a wallet to get started"),
    ).toBeInTheDocument();
  });

  it("does not render description when omitted", () => {
    const { container } = render(<EmptyState title="Empty" />);
    const descriptions = container.querySelectorAll(".text-muted-foreground");
    expect(descriptions).toHaveLength(0);
  });

  it("renders icon when provided", () => {
    const { container } = render(
      <EmptyState title="Empty" icon={Wallet} />,
    );
    // Lucide renders an SVG element
    const svg = container.querySelector("svg");
    expect(svg).toBeInTheDocument();
    expect(svg).toHaveAttribute("aria-hidden", "true");
  });

  it("does not render icon when omitted", () => {
    const { container } = render(<EmptyState title="Empty" />);
    expect(container.querySelector("svg")).not.toBeInTheDocument();
  });

  it("renders action button when both label and handler provided", () => {
    const onAction = vi.fn();
    render(
      <EmptyState
        title="Empty"
        actionLabel="Create Wallet"
        onAction={onAction}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Create Wallet" }),
    ).toBeInTheDocument();
  });

  it("calls onAction when button is clicked", async () => {
    const user = userEvent.setup();
    const onAction = vi.fn();
    render(
      <EmptyState
        title="Empty"
        actionLabel="Create Wallet"
        onAction={onAction}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Create Wallet" }));
    expect(onAction).toHaveBeenCalledOnce();
  });

  it("does not render button when actionLabel is missing", () => {
    render(<EmptyState title="Empty" onAction={() => {}} />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("does not render button when onAction is missing", () => {
    render(<EmptyState title="Empty" actionLabel="Click me" />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("has status role for accessibility", () => {
    render(<EmptyState title="Empty" />);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(
      <EmptyState title="Empty" className="my-custom" />,
    );
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("my-custom");
  });
});
