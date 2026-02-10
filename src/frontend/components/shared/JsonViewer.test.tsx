import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { JsonViewer } from "./JsonViewer";

// Mock useTheme — inline mock matching the original pattern.
// Cannot use centralized createThemeMock here because render-helpers.tsx
// imports from @/components/theme/ThemeProvider, causing hoisting issues.
vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({ resolvedTheme: "light", theme: "light", setTheme: () => {} }),
}));

describe("JsonViewer", () => {
  it("renders JSON object data", () => {
    const data = { name: "Alice", balance: 100 };
    render(<JsonViewer data={data} />);
    expect(screen.getByText(/Alice/)).toBeInTheDocument();
  });

  it("renders string data as pre-formatted text", () => {
    render(<JsonViewer data="simple string" />);
    expect(screen.getByText(/simple string/)).toBeInTheDocument();
  });

  it("renders JSON string data parsed into tree view", () => {
    const jsonStr = '{"key": "value"}';
    render(<JsonViewer data={jsonStr} />);
    expect(screen.getByText(/key/)).toBeInTheDocument();
  });

  it("renders array data", () => {
    const data = [1, 2, 3];
    render(<JsonViewer data={data} />);
    expect(screen.getByText(/1/)).toBeInTheDocument();
  });

  it("shows copy button by default", () => {
    render(<JsonViewer data={{ test: true }} />);
    expect(
      screen.getByRole("button", { name: "Copy to clipboard" }),
    ).toBeInTheDocument();
  });

  it("hides copy button when showCopy is false", () => {
    render(<JsonViewer data={{ test: true }} showCopy={false} />);
    expect(
      screen.queryByRole("button", { name: "Copy to clipboard" }),
    ).not.toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(
      <JsonViewer data={{ a: 1 }} className="my-class" />,
    );
    expect(container.firstChild).toHaveClass("my-class");
  });

  it("renders nested objects", () => {
    const data = {
      user: {
        name: "Bob",
        wallet: {
          balance: 50,
        },
      },
    };
    render(<JsonViewer data={data} />);
    expect(screen.getByText(/Bob/)).toBeInTheDocument();
  });

  it("handles null data gracefully", () => {
    render(<JsonViewer data={null} />);
    expect(screen.getByText(/null/)).toBeInTheDocument();
  });

  it("has monospace font styling", () => {
    const { container } = render(<JsonViewer data={{ x: 1 }} />);
    expect(container.firstChild).toHaveClass("font-mono");
  });
});
