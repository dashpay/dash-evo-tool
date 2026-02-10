import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import App from "./App";
import { ThemeProvider } from "@/components/theme";

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

describe("App", () => {
  it("renders the main heading and IPC test card", () => {
    render(
      <ThemeProvider>
        <App />
      </ThemeProvider>,
    );

    expect(
      screen.getByRole("heading", { name: /dash evo tool/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/tauri 2\.0 \+ react frontend/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/ipc test/i)).toBeInTheDocument();
    expect(
      screen.getByPlaceholderText(/enter your name/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /greet/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /get app version/i }),
    ).toBeInTheDocument();
  });
});
