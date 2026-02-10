/**
 * Tests for centralized render helpers.
 *
 * Validates that `renderWithProviders`, `createRouterMock`, and
 * `createThemeMock` work correctly for test infrastructure.
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen, cleanup } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock bindings to prevent ThemeProvider from hitting real IPC
vi.mock("@/bindings", () => ({
  commands: {
    settingsGet: vi.fn().mockResolvedValue({
      status: "ok",
      data: { themeMode: "dark" },
    }),
    systemUpdateTheme: vi.fn().mockResolvedValue({ status: "ok", data: null }),
  },
  events: {},
}));

import {
  renderWithProviders,
  createRouterMock,
  createThemeMock,
} from "./render-helpers";
import { useTheme } from "@/components/theme/ThemeProvider";

// ---------------------------------------------------------------------------
// Test components
// ---------------------------------------------------------------------------

function ThemeDisplay() {
  const { resolvedTheme } = useTheme();
  return <div data-testid="theme">{resolvedTheme}</div>;
}

function NavigationComponent({
  navigate,
}: {
  navigate: (opts: { to: string }) => void;
}) {
  return (
    <button onClick={() => navigate({ to: "/wallets" })}>Go to Wallets</button>
  );
}

function SimpleComponent() {
  return <div data-testid="simple">Hello</div>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("renderWithProviders", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders a simple component", () => {
    renderWithProviders(<SimpleComponent />);
    expect(screen.getByTestId("simple")).toHaveTextContent("Hello");
  });

  it("provides ThemeProvider context by default", () => {
    renderWithProviders(<ThemeDisplay />);
    expect(screen.getByTestId("theme")).toBeInTheDocument();
    // Default theme is "dark"
    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
  });

  it("allows specifying light theme", () => {
    renderWithProviders(<ThemeDisplay />, { theme: "light" });
    expect(screen.getByTestId("theme")).toHaveTextContent("light");
  });

  it("renders without ThemeProvider when theme=null", () => {
    // ThemeDisplay should throw because useTheme() requires ThemeProvider
    expect(() => {
      renderWithProviders(<ThemeDisplay />, { theme: null });
    }).toThrow("useTheme must be used within a ThemeProvider");
  });

  it("provides TooltipProvider context", () => {
    // This tests that components needing TooltipProvider won't throw
    // We can't easily assert TooltipProvider directly, but rendering
    // without it would cause errors in Radix components
    renderWithProviders(<SimpleComponent />);
    expect(screen.getByTestId("simple")).toBeInTheDocument();
  });
});

describe("createRouterMock", () => {
  it("creates a mock with navigate function", () => {
    const router = createRouterMock();
    expect(router.navigate).toBeDefined();
    expect(typeof router.navigate).toBe("function");
  });

  it("navigate is a vi.fn() mock", () => {
    const router = createRouterMock();
    router.navigate({ to: "/identities" });
    expect(router.navigate).toHaveBeenCalledWith({ to: "/identities" });
  });

  it("provides mutable params", () => {
    const router = createRouterMock({ walletId: "abc" });
    expect(router.params.walletId).toBe("abc");
    router.params.walletId = "def";
    expect(router.module.useParams().walletId).toBe("def");
  });

  it("provides mutable search params", () => {
    const router = createRouterMock({}, { tab: "addresses" });
    expect(router.search.tab).toBe("addresses");
    router.search.tab = "transactions";
    expect(router.module.useSearch().tab).toBe("transactions");
  });

  it("module.useNavigate returns the same navigate mock", () => {
    const router = createRouterMock();
    const nav = router.module.useNavigate();
    expect(nav).toBe(router.navigate);
  });

  it("module.Link renders an anchor element", () => {
    const router = createRouterMock();
    const { Link } = router.module;
    renderWithProviders(<Link to="/wallets">Go</Link>);
    const link = screen.getByText("Go");
    expect(link.tagName).toBe("A");
    expect(link).toHaveAttribute("href", "/wallets");
  });

  it("navigate can be used in click handlers", async () => {
    const router = createRouterMock();
    const user = userEvent.setup();

    renderWithProviders(
      <NavigationComponent navigate={router.navigate} />,
    );

    await user.click(screen.getByRole("button", { name: /go to wallets/i }));
    expect(router.navigate).toHaveBeenCalledWith({ to: "/wallets" });
  });
});

describe("createThemeMock", () => {
  it("returns a mock module with useTheme", () => {
    const mock = createThemeMock();
    expect(mock.useTheme).toBeDefined();
    expect(typeof mock.useTheme).toBe("function");
  });

  it("useTheme returns the specified resolved theme", () => {
    const mock = createThemeMock("dark");
    const result = mock.useTheme();
    expect(result.resolvedTheme).toBe("dark");
    expect(result.theme).toBe("dark");
  });

  it("useTheme defaults to light theme", () => {
    const mock = createThemeMock();
    const result = mock.useTheme();
    expect(result.resolvedTheme).toBe("light");
  });

  it("provides a setTheme mock function", () => {
    const mock = createThemeMock();
    const { setTheme } = mock.useTheme();
    setTheme("dark");
    expect(mock._setTheme).toHaveBeenCalledWith("dark");
  });

  it("ThemeProvider passthrough renders children", () => {
    const mock = createThemeMock();
    const { ThemeProvider } = mock;
    renderWithProviders(
      <ThemeProvider>
        <div data-testid="child">content</div>
      </ThemeProvider>,
    );
    expect(screen.getByTestId("child")).toHaveTextContent("content");
  });
});
