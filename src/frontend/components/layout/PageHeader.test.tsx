import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { PageHeader } from "./PageHeader";

describe("PageHeader", () => {
  it("renders the title", () => {
    render(<PageHeader title="Wallets" />);
    expect(
      screen.getByRole("heading", { name: "Wallets", level: 1 }),
    ).toBeInTheDocument();
  });

  it("renders subtitle when provided", () => {
    render(<PageHeader title="Wallets" subtitle="Manage your wallets" />);
    expect(screen.getByText("Manage your wallets")).toBeInTheDocument();
  });

  it("does not render subtitle when not provided", () => {
    const { container } = render(<PageHeader title="Wallets" />);
    const subtitle = container.querySelector("p");
    expect(subtitle).toBeNull();
  });

  it("renders breadcrumbs with separators", () => {
    render(
      <PageHeader
        title="Details"
        breadcrumbs={[{ label: "Wallets" }, { label: "HD Wallet #1" }]}
      />,
    );

    const nav = screen.getByRole("navigation", { name: /breadcrumb/i });
    expect(nav).toBeInTheDocument();
    expect(screen.getByText("Wallets")).toBeInTheDocument();
    expect(screen.getByText("HD Wallet #1")).toBeInTheDocument();
  });

  it("does not render breadcrumb nav when breadcrumbs are empty", () => {
    render(<PageHeader title="Wallets" breadcrumbs={[]} />);
    expect(
      screen.queryByRole("navigation", { name: /breadcrumb/i }),
    ).not.toBeInTheDocument();
  });

  it("renders clickable breadcrumbs as buttons", async () => {
    const user = userEvent.setup();
    const handleClick = vi.fn();

    render(
      <PageHeader
        title="HD Wallet #1"
        breadcrumbs={[
          { label: "Wallets", onClick: handleClick },
          { label: "HD Wallet #1" },
        ]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Wallets" }));
    expect(handleClick).toHaveBeenCalledOnce();
  });

  it("renders non-clickable breadcrumbs as plain text", () => {
    render(
      <PageHeader
        title="Details"
        breadcrumbs={[{ label: "Home" }]}
      />,
    );

    expect(screen.queryByRole("button", { name: "Home" })).not.toBeInTheDocument();
    expect(screen.getByText("Home")).toBeInTheDocument();
  });

  it("renders action buttons on the right", () => {
    render(
      <PageHeader
        title="Identities"
        actions={<button data-testid="add-btn">Add</button>}
      />,
    );

    expect(screen.getByTestId("add-btn")).toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(
      <PageHeader title="Test" className="mt-4" />,
    );

    expect(container.firstChild).toHaveClass("mt-4");
  });
});
