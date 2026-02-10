/**
 * Centralized render helpers for Vitest + React Testing Library.
 *
 * These helpers reduce boilerplate in test files by:
 * 1. Wrapping rendered components in common providers (TooltipProvider, ThemeProvider)
 * 2. Providing pre-configured router and theme mocks
 * 3. Integrating with the centralized mock IPC infrastructure
 *
 * ## USAGE PATTERNS
 *
 * ### Pattern A: Screen test with IPC + router mocks
 *
 *   ```ts
 *   // Use vi.hoisted for navigate mock (needed in vi.mock factory)
 *   const { mockNavigate } = vi.hoisted(() => ({ mockNavigate: vi.fn() }));
 *
 *   vi.mock("@tanstack/react-router", () => ({
 *     useNavigate: () => mockNavigate,
 *   }));
 *
 *   // Use async factory with dynamic import for centralized bindings mock
 *   vi.mock("@/bindings", async () => {
 *     const { createMockBindings, mockBindingsModule } = await import("@/test/mock-ipc");
 *     return mockBindingsModule(createMockBindings());
 *   });
 *
 *   import { commands } from "@/bindings";
 *   // commands.* are all vi.fn() mocks with default responses
 *   ```
 *
 * ### Pattern B: Component test with providers
 *
 *   ```ts
 *   import { renderWithProviders } from "@/test/render-helpers";
 *   renderWithProviders(<MyComponent />);
 *   renderWithProviders(<MyComponent />, { theme: "light" });
 *   ```
 *
 * ### Pattern C: Store test with centralized fixtures
 *
 *   ```ts
 *   import { createMockBindings, mockBindingsModule } from "@/test/mock-ipc";
 *   import { createMockContract } from "@/test/fixtures";
 *
 *   vi.mock("../bindings", () => {
 *     const initial = createMockBindings();
 *     return mockBindingsModule(initial);
 *   });
 *   ```
 */

import { render, type RenderOptions } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ThemeProvider } from "@/components/theme/ThemeProvider";
import { vi, type Mock } from "vitest";
import type { ReactElement } from "react";

// ---------------------------------------------------------------------------
// Provider wrapper
// ---------------------------------------------------------------------------

interface RenderWithProvidersOptions extends Omit<RenderOptions, "wrapper"> {
  /**
   * Initial theme for the ThemeProvider. Defaults to "dark".
   * Set to `null` to skip ThemeProvider wrapping.
   */
  theme?: "light" | "dark" | "system" | null;
}

/**
 * Renders a component wrapped in common providers needed by most components:
 * - TooltipProvider (required by Radix UI tooltips / Sidebar)
 * - ThemeProvider (required by components using `useTheme()`)
 *
 * @example
 * ```ts
 * renderWithProviders(<WelcomeScreen />);
 * renderWithProviders(<MyComponent />, { theme: "light" });
 * renderWithProviders(<MyComponent />, { theme: null }); // no ThemeProvider
 * ```
 */
export function renderWithProviders(
  ui: ReactElement,
  { theme = "dark", ...renderOptions }: RenderWithProvidersOptions = {},
) {
  function Wrapper({ children }: { children: React.ReactNode }) {
    if (theme === null) {
      return <TooltipProvider>{children}</TooltipProvider>;
    }
    return (
      <ThemeProvider defaultTheme={theme}>
        <TooltipProvider>{children}</TooltipProvider>
      </ThemeProvider>
    );
  }

  return render(ui, { wrapper: Wrapper, ...renderOptions });
}

// ---------------------------------------------------------------------------
// Router mock factory
// ---------------------------------------------------------------------------

export interface RouterMockResult {
  /** The mock `useNavigate()` function — assert calls with `expect(router.navigate)` */
  navigate: Mock;
  /** Mock params returned by `useParams()`. Set before rendering. */
  params: Record<string, string>;
  /** Mock search params returned by `useSearch()`. Set before rendering. */
  search: Record<string, unknown>;
  /** The mock module — pass to `vi.mock("@tanstack/react-router", () => router.module)` */
  module: {
    useNavigate: () => Mock;
    useParams: () => Record<string, string>;
    useSearch: () => Record<string, unknown>;
    useRouter: () => { history: { back: Mock } };
    Link: (props: { children: React.ReactNode; to: string }) => ReactElement;
  };
}

/**
 * Creates a reusable router mock for `@tanstack/react-router`.
 *
 * Returns an object with:
 * - `navigate`: the mock function (for assertions)
 * - `params`: mutable params object (set before render)
 * - `search`: mutable search params object (set before render)
 * - `module`: the mock module object to pass to `vi.mock()`
 *
 * @example
 * ```ts
 * const router = createRouterMock();
 * vi.mock("@tanstack/react-router", () => router.module);
 *
 * // Set params before rendering
 * router.params.walletId = "abc123";
 *
 * renderWithProviders(<WalletDetail />);
 * expect(router.navigate).toHaveBeenCalledWith({ to: "/wallets" });
 * ```
 */
export function createRouterMock(
  initialParams: Record<string, string> = {},
  initialSearch: Record<string, unknown> = {},
): RouterMockResult {
  const navigate = vi.fn();
  const historyBack = vi.fn();
  const params = { ...initialParams };
  const search = { ...initialSearch };

  const module = {
    useNavigate: () => navigate,
    useParams: () => params,
    useSearch: () => search,
    useRouter: () => ({ history: { back: historyBack } }),
    Link: ({
      children,
      to,
      ...rest
    }: {
      children: React.ReactNode;
      to: string;
      [key: string]: unknown;
    }) => (
      <a href={to} data-testid={`link-${to}`} {...rest}>
        {children}
      </a>
    ),
  };

  return { navigate, params, search, module };
}

// ---------------------------------------------------------------------------
// Theme mock factory
// ---------------------------------------------------------------------------

/**
 * Creates a mock module for `@/components/theme/ThemeProvider`.
 * Use when a component calls `useTheme()` and you don't want to wrap
 * with a real ThemeProvider.
 *
 * @example
 * ```ts
 * vi.mock("@/components/theme/ThemeProvider", () => createThemeMock("dark"));
 * ```
 */
export function createThemeMock(resolvedTheme: "light" | "dark" = "light") {
  const setTheme = vi.fn();
  return {
    useTheme: () => ({
      resolvedTheme,
      theme: resolvedTheme,
      setTheme,
    }),
    ThemeProvider: ({ children }: { children: React.ReactNode }) => (
      <>{children}</>
    ),
    _setTheme: setTheme,
  };
}

// ---------------------------------------------------------------------------
// Re-export existing helpers for convenience
// ---------------------------------------------------------------------------

export { renderWithProviders as renderWithTooltipProvider } from "./router-utils";
