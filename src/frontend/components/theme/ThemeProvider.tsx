import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import type { ThemeModeDto } from "@/bindings";

type ResolvedTheme = "light" | "dark";

interface ThemeContextValue {
  /** The user's theme preference: "light", "dark", or "system" */
  theme: ThemeModeDto;
  /** The resolved theme after applying system preference */
  resolvedTheme: ResolvedTheme;
  /** Update the theme preference. Persists to backend if available. */
  setTheme: (theme: ThemeModeDto) => void;
}

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

function getSystemTheme(): ResolvedTheme {
  if (typeof window === "undefined") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

function resolveTheme(theme: ThemeModeDto): ResolvedTheme {
  return theme === "system" ? getSystemTheme() : theme;
}

function applyThemeToDOM(resolved: ResolvedTheme) {
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(resolved);
}

interface ThemeProviderProps {
  children: React.ReactNode;
  /** Initial theme to use before backend settings are loaded */
  defaultTheme?: ThemeModeDto;
}

export function ThemeProvider({
  children,
  defaultTheme = "system",
}: ThemeProviderProps) {
  const [theme, setThemeState] = useState<ThemeModeDto>(defaultTheme);
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveTheme(defaultTheme),
  );

  // Load initial theme from backend settings
  useEffect(() => {
    let cancelled = false;
    async function loadTheme() {
      try {
        const { commands } = await import("@/bindings");
        const result = await commands.settingsGet();
        if (cancelled) return;
        if (result.status === "ok") {
          setThemeState(result.data.themeMode);
        }
      } catch {
        // Backend not available (browser-only mode) — keep default
      }
    }
    loadTheme();
    return () => {
      cancelled = true;
    };
  }, []);

  // Apply resolved theme to DOM whenever theme changes
  useEffect(() => {
    const resolved = resolveTheme(theme);
    setResolvedTheme(resolved);
    applyThemeToDOM(resolved);
  }, [theme]);

  // Listen for system theme changes when using "system" mode
  useEffect(() => {
    if (theme !== "system") return;

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = () => {
      const resolved = resolveTheme("system");
      setResolvedTheme(resolved);
      applyThemeToDOM(resolved);
    };

    mediaQuery.addEventListener("change", handleChange);
    return () => mediaQuery.removeEventListener("change", handleChange);
  }, [theme]);

  const setTheme = useCallback((newTheme: ThemeModeDto) => {
    setThemeState(newTheme);

    // Persist to backend (fire-and-forget)
    import("@/bindings")
      .then(({ commands }) => commands.systemUpdateTheme(newTheme))
      .catch(() => {
        // Backend not available — theme still applied locally
      });
  }, []);

  const value = useMemo(
    () => ({ theme, resolvedTheme, setTheme }),
    [theme, resolvedTheme, setTheme],
  );

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (context === undefined) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }
  return context;
}
