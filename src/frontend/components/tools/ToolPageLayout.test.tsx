import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { ToolPageLayout } from "./ToolPageLayout";

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

describe("ToolPageLayout", () => {
  it("renders the title", () => {
    render(<ToolPageLayout title="Platform Info">Content</ToolPageLayout>);
    expect(
      screen.getByRole("heading", { name: "Platform Info" }),
    ).toBeInTheDocument();
  });

  it("renders children content", () => {
    render(
      <ToolPageLayout title="Test">
        <p>Tool content here</p>
      </ToolPageLayout>,
    );
    expect(screen.getByText("Tool content here")).toBeInTheDocument();
  });

  it("renders subtitle when provided", () => {
    render(
      <ToolPageLayout title="Test" subtitle="A helpful description">
        Content
      </ToolPageLayout>,
    );
    expect(screen.getByText("A helpful description")).toBeInTheDocument();
  });

  it("renders action buttons when provided", () => {
    render(
      <ToolPageLayout
        title="Test"
        actions={<button type="button">Fetch</button>}
      >
        Content
      </ToolPageLayout>,
    );
    expect(screen.getByRole("button", { name: "Fetch" })).toBeInTheDocument();
  });

  it("shows back button by default", () => {
    render(<ToolPageLayout title="Test">Content</ToolPageLayout>);
    expect(
      screen.getByRole("button", { name: "Back to Tools" }),
    ).toBeInTheDocument();
  });

  it("hides back button when showBack is false", () => {
    render(
      <ToolPageLayout title="Test" showBack={false}>
        Content
      </ToolPageLayout>,
    );
    expect(
      screen.queryByRole("button", { name: "Back to Tools" }),
    ).not.toBeInTheDocument();
  });

  it("navigates to /tools when back button is clicked", async () => {
    const user = userEvent.setup();
    render(<ToolPageLayout title="Test">Content</ToolPageLayout>);
    await user.click(screen.getByRole("button", { name: "Back to Tools" }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tools" });
  });

  it("applies custom className", () => {
    const { container } = render(
      <ToolPageLayout title="Test" className="my-custom">
        Content
      </ToolPageLayout>,
    );
    expect(container.firstChild).toHaveClass("my-custom");
  });

  it("wraps content in an Island", () => {
    const { container } = render(
      <ToolPageLayout title="Test">Content</ToolPageLayout>,
    );
    expect(container.querySelector(".island")).toBeInTheDocument();
  });
});
