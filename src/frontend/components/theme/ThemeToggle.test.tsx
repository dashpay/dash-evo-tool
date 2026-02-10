import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { createMockBindings, mockBindingsModule } from "@/test/mock-ipc";
import { ThemeProvider } from "./ThemeProvider";
import { ThemeToggle } from "./ThemeToggle";

// Mock the bindings module using centralized mock infrastructure
vi.mock("@/bindings", () => {
  const initial = createMockBindings();
  return mockBindingsModule(initial);
});

beforeEach(() => {
  document.documentElement.classList.remove("light", "dark");
});

function renderWithTheme(defaultTheme: "light" | "dark" | "system" = "light") {
  return render(
    <ThemeProvider defaultTheme={defaultTheme}>
      <ThemeToggle />
    </ThemeProvider>,
  );
}

describe("ThemeToggle", () => {
  it("renders a toggle button", () => {
    renderWithTheme();
    expect(screen.getByRole("button", { name: /toggle theme/i })).toBeInTheDocument();
  });

  it("opens dropdown menu on click", async () => {
    const user = userEvent.setup();
    renderWithTheme();

    await user.click(screen.getByRole("button", { name: /toggle theme/i }));

    expect(screen.getByText("Light")).toBeInTheDocument();
    expect(screen.getByText("Dark")).toBeInTheDocument();
    expect(screen.getByText("System")).toBeInTheDocument();
  });

  it("switches to dark mode when Dark is selected", async () => {
    const user = userEvent.setup();
    renderWithTheme("light");

    await user.click(screen.getByRole("button", { name: /toggle theme/i }));
    await user.click(screen.getByText("Dark"));

    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("switches to light mode when Light is selected", async () => {
    const user = userEvent.setup();
    renderWithTheme("dark");

    await user.click(screen.getByRole("button", { name: /toggle theme/i }));
    await user.click(screen.getByText("Light"));

    expect(document.documentElement.classList.contains("light")).toBe(true);
  });

  it("calls systemUpdateTheme on selection", async () => {
    const user = userEvent.setup();
    const { commands } = await import("@/bindings");

    renderWithTheme("light");

    await user.click(screen.getByRole("button", { name: /toggle theme/i }));
    await user.click(screen.getByText("Dark"));

    expect(commands.systemUpdateTheme).toHaveBeenCalledWith("dark");
  });

  it("has accessible button with aria-label", () => {
    renderWithTheme();
    const button = screen.getByRole("button", { name: /toggle theme/i });
    expect(button).toHaveAttribute("aria-label", "Toggle theme");
  });
});
