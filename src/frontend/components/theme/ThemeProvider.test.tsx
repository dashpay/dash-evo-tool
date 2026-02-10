import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { createMockBindings, mockBindingsModule } from "@/test/mock-ipc";
import { createMockSettings } from "@/test/fixtures";
import { ThemeProvider, useTheme } from "./ThemeProvider";

// Mock the bindings module using centralized mock infrastructure
vi.mock("@/bindings", () => {
  const initial = createMockBindings();
  return mockBindingsModule(initial);
});

// Helper component to expose theme context in tests
function ThemeConsumer() {
  const { theme, resolvedTheme, setTheme } = useTheme();
  return (
    <div>
      <span data-testid="theme">{theme}</span>
      <span data-testid="resolved">{resolvedTheme}</span>
      <button onClick={() => setTheme("dark")}>Set Dark</button>
      <button onClick={() => setTheme("light")}>Set Light</button>
      <button onClick={() => setTheme("system")}>Set System</button>
    </div>
  );
}

function mockMatchMedia(prefersDark: boolean) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: query === "(prefers-color-scheme: dark)" ? prefersDark : false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
}

beforeEach(() => {
  document.documentElement.classList.remove("light", "dark");
  mockMatchMedia(false);
});

describe("ThemeProvider", () => {
  it("renders children", () => {
    render(
      <ThemeProvider>
        <span>Hello</span>
      </ThemeProvider>,
    );
    expect(screen.getByText("Hello")).toBeInTheDocument();
  });

  it("provides default theme as system", () => {
    render(
      <ThemeProvider>
        <ThemeConsumer />
      </ThemeProvider>,
    );
    expect(screen.getByTestId("theme").textContent).toBe("system");
  });

  it("accepts a custom default theme", () => {
    render(
      <ThemeProvider defaultTheme="dark">
        <ThemeConsumer />
      </ThemeProvider>,
    );
    expect(screen.getByTestId("theme").textContent).toBe("dark");
  });

  it("resolves system theme to light when system prefers light", () => {
    render(
      <ThemeProvider defaultTheme="system">
        <ThemeConsumer />
      </ThemeProvider>,
    );
    expect(screen.getByTestId("resolved").textContent).toBe("light");
  });

  it("resolves system theme to dark when system prefers dark", () => {
    mockMatchMedia(true);

    render(
      <ThemeProvider defaultTheme="system">
        <ThemeConsumer />
      </ThemeProvider>,
    );
    expect(screen.getByTestId("resolved").textContent).toBe("dark");
  });

  it("applies dark class to document when theme is dark", () => {
    render(
      <ThemeProvider defaultTheme="dark">
        <ThemeConsumer />
      </ThemeProvider>,
    );
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.classList.contains("light")).toBe(false);
  });

  it("applies light class to document when theme is light", () => {
    render(
      <ThemeProvider defaultTheme="light">
        <ThemeConsumer />
      </ThemeProvider>,
    );
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("updates theme when setTheme is called", async () => {
    const user = userEvent.setup();

    render(
      <ThemeProvider defaultTheme="light">
        <ThemeConsumer />
      </ThemeProvider>,
    );

    expect(screen.getByTestId("theme").textContent).toBe("light");

    await user.click(screen.getByText("Set Dark"));

    expect(screen.getByTestId("theme").textContent).toBe("dark");
    expect(screen.getByTestId("resolved").textContent).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("persists theme to backend when setTheme is called", async () => {
    const user = userEvent.setup();
    const { commands } = await import("@/bindings");

    render(
      <ThemeProvider defaultTheme="light">
        <ThemeConsumer />
      </ThemeProvider>,
    );

    await user.click(screen.getByText("Set Dark"));

    expect(commands.systemUpdateTheme).toHaveBeenCalledWith("dark");
  });

  it("loads theme from backend settings on mount", async () => {
    const { commands } = await import("@/bindings");
    vi.mocked(commands.settingsGet).mockResolvedValueOnce({
      status: "ok",
      data: createMockSettings({ themeMode: "dark" }),
    });

    render(
      <ThemeProvider defaultTheme="system">
        <ThemeConsumer />
      </ThemeProvider>,
    );

    // Wait for async settings load
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
    });

    expect(screen.getByTestId("theme").textContent).toBe("dark");
  });

  it("listens for system theme changes when in system mode", () => {
    const listeners: Array<() => void> = [];
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn((_event: string, fn: () => void) => {
          listeners.push(fn);
        }),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });

    render(
      <ThemeProvider defaultTheme="system">
        <ThemeConsumer />
      </ThemeProvider>,
    );

    expect(listeners.length).toBeGreaterThan(0);
  });

  it("does not listen for system changes when not in system mode", () => {
    const addEventListener = vi.fn();
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener,
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });

    render(
      <ThemeProvider defaultTheme="dark">
        <ThemeConsumer />
      </ThemeProvider>,
    );

    expect(addEventListener).not.toHaveBeenCalled();
  });
});

describe("useTheme", () => {
  it("throws when used outside ThemeProvider", () => {
    // Suppress React error boundary console output
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});

    expect(() => {
      render(<ThemeConsumer />);
    }).toThrow("useTheme must be used within a ThemeProvider");

    spy.mockRestore();
  });
});
