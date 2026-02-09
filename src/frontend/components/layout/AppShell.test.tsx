import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { AppShell } from "./AppShell";

describe("AppShell", () => {
  it("renders sidebar and content children", () => {
    render(
      <AppShell sidebar={<div data-testid="sidebar">Nav</div>}>
        <div data-testid="content">Page Content</div>
      </AppShell>,
    );

    expect(screen.getByTestId("sidebar")).toBeInTheDocument();
    expect(screen.getByTestId("content")).toBeInTheDocument();
  });

  it("renders sidebar in a navigation landmark", () => {
    render(
      <AppShell sidebar={<div>Nav</div>}>
        <div>Content</div>
      </AppShell>,
    );

    const nav = screen.getByRole("navigation", { name: /main navigation/i });
    expect(nav).toBeInTheDocument();
  });

  it("renders content in a main landmark", () => {
    render(
      <AppShell sidebar={<div>Nav</div>}>
        <div>Content</div>
      </AppShell>,
    );

    const main = screen.getByRole("main");
    expect(main).toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(
      <AppShell sidebar={<div>Nav</div>} className="custom-class">
        <div>Content</div>
      </AppShell>,
    );

    expect(container.firstChild).toHaveClass("custom-class");
  });

  it("uses full viewport dimensions", () => {
    const { container } = render(
      <AppShell sidebar={<div>Nav</div>}>
        <div>Content</div>
      </AppShell>,
    );

    const root = container.firstChild as HTMLElement;
    expect(root).toHaveClass("h-screen", "w-screen");
  });
});
