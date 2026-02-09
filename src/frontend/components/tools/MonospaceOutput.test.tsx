import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { MonospaceOutput } from "./MonospaceOutput";

// Mock the CopyButton since it has clipboard dependencies
vi.mock("@/components/shared/CopyButton", () => ({
  CopyButton: ({ value, label }: { value: string; label?: string }) => (
    <button type="button" aria-label={`Copy: ${value.substring(0, 20)}`}>
      {label || "Copy"}
    </button>
  ),
}));

describe("MonospaceOutput", () => {
  it("renders the output value", () => {
    render(<MonospaceOutput value="Hello World" />);
    expect(screen.getByText("Hello World")).toBeInTheDocument();
  });

  it("renders in a pre tag with monospace styling", () => {
    const { container } = render(<MonospaceOutput value="test output" />);
    const pre = container.querySelector("pre");
    expect(pre).toBeInTheDocument();
    expect(pre).toHaveTextContent("test output");
  });

  it("renders with a label", () => {
    render(<MonospaceOutput value="data" label="Result" />);
    expect(screen.getByText("Result")).toBeInTheDocument();
  });

  it("shows copy button by default when value is present", () => {
    render(<MonospaceOutput value="data" />);
    expect(screen.getByText("Copy")).toBeInTheDocument();
  });

  it("hides copy button when showCopy is false", () => {
    render(<MonospaceOutput value="data" showCopy={false} />);
    expect(screen.queryByText("Copy")).not.toBeInTheDocument();
  });

  it("does not show copy button when value is empty", () => {
    render(<MonospaceOutput value="" />);
    expect(screen.queryByText("Copy")).not.toBeInTheDocument();
  });

  it("shows placeholder text when value is empty", () => {
    render(<MonospaceOutput value="" />);
    expect(screen.getByText("No output")).toBeInTheDocument();
  });

  it("applies word-wrap by default", () => {
    const { container } = render(<MonospaceOutput value="long text" />);
    const pre = container.querySelector("pre");
    expect(pre).toHaveClass("whitespace-pre-wrap", "break-all");
  });

  it("disables word-wrap when wrap is false", () => {
    const { container } = render(
      <MonospaceOutput value="long text" wrap={false} />,
    );
    const pre = container.querySelector("pre");
    expect(pre).toHaveClass("whitespace-pre");
    expect(pre).not.toHaveClass("whitespace-pre-wrap");
  });

  it("applies max-height style", () => {
    const { container } = render(
      <MonospaceOutput value="data" maxHeight={200} />,
    );
    const outputDiv = container.querySelector('[role="log"]');
    expect(outputDiv).toHaveStyle({ maxHeight: "200px" });
  });

  it("has accessible role", () => {
    render(<MonospaceOutput value="test" label="Output" />);
    expect(screen.getByRole("log")).toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(
      <MonospaceOutput value="test" className="my-class" />,
    );
    expect(container.firstChild).toHaveClass("my-class");
  });
});
