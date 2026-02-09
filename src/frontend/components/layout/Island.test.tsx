import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { Island } from "./Island";

describe("Island", () => {
  it("renders children", () => {
    render(<Island>Hello Island</Island>);
    expect(screen.getByText("Hello Island")).toBeInTheDocument();
  });

  it("applies island class with elevated styling", () => {
    const { container } = render(<Island>Content</Island>);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("island");
  });

  it("applies default padding", () => {
    const { container } = render(<Island>Content</Island>);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("p-6");
  });

  it("removes padding when noPadding is true", () => {
    const { container } = render(<Island noPadding>Content</Island>);
    const el = container.firstChild as HTMLElement;
    expect(el).not.toHaveClass("p-6");
  });

  it("renders as div by default", () => {
    const { container } = render(<Island>Content</Island>);
    expect(container.firstChild?.nodeName).toBe("DIV");
  });

  it("renders as section when specified", () => {
    const { container } = render(<Island as="section">Content</Island>);
    expect(container.firstChild?.nodeName).toBe("SECTION");
  });

  it("renders as article when specified", () => {
    const { container } = render(<Island as="article">Content</Island>);
    expect(container.firstChild?.nodeName).toBe("ARTICLE");
  });

  it("applies custom className", () => {
    const { container } = render(
      <Island className="my-custom">Content</Island>,
    );
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("island", "my-custom");
  });
});
