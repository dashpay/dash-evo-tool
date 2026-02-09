import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { WelcomeScreen } from "./WelcomeScreen";

// Mock @tanstack/react-router
const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

// Mock Tauri bindings
vi.mock("@/bindings", () => ({
  commands: {
    settingsUpdateOnboardingCompleted: vi.fn().mockResolvedValue({ status: "ok", data: null }),
  },
}));

import { commands } from "@/bindings";

describe("WelcomeScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the Dash logo", () => {
    render(<WelcomeScreen />);
    expect(screen.getByAltText("Dash")).toBeInTheDocument();
  });

  it("renders the welcome title", () => {
    render(<WelcomeScreen />);
    expect(screen.getByRole("heading", { name: /welcome to dash evo tool/i })).toBeInTheDocument();
  });

  it("renders the subtitle", () => {
    render(<WelcomeScreen />);
    expect(screen.getByText(/your gateway to decentralized data/i)).toBeInTheDocument();
  });

  it("renders the instructional text", () => {
    render(<WelcomeScreen />);
    expect(screen.getByText(/select an option to get started/i)).toBeInTheDocument();
  });

  it("renders three action cards", () => {
    render(<WelcomeScreen />);
    expect(screen.getByRole("button", { name: /create wallet/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /import wallet/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /just explore/i })).toBeInTheDocument();
  });

  it("renders card descriptions", () => {
    render(<WelcomeScreen />);
    expect(screen.getByText("Start fresh with a new HD wallet")).toBeInTheDocument();
    expect(screen.getByText("Load a wallet you already have")).toBeInTheDocument();
    expect(screen.getByText("Explore without setting up")).toBeInTheDocument();
  });

  it("clicking Create Wallet marks onboarding complete and navigates to /wallets", async () => {
    render(<WelcomeScreen />);

    fireEvent.click(screen.getByRole("button", { name: /create wallet/i }));

    await waitFor(() => {
      expect(commands.settingsUpdateOnboardingCompleted).toHaveBeenCalledWith(true);
    });
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  it("clicking Import Wallet marks onboarding complete and navigates to /wallets", async () => {
    render(<WelcomeScreen />);

    fireEvent.click(screen.getByRole("button", { name: /import wallet/i }));

    await waitFor(() => {
      expect(commands.settingsUpdateOnboardingCompleted).toHaveBeenCalledWith(true);
    });
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  it("clicking Just Explore marks onboarding complete and navigates to /identities", async () => {
    render(<WelcomeScreen />);

    fireEvent.click(screen.getByRole("button", { name: /just explore/i }));

    await waitFor(() => {
      expect(commands.settingsUpdateOnboardingCompleted).toHaveBeenCalledWith(true);
    });
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
  });

  it("handles backend error gracefully (still navigates)", async () => {
    vi.mocked(commands.settingsUpdateOnboardingCompleted).mockRejectedValueOnce(new Error("no backend"));

    render(<WelcomeScreen />);

    fireEvent.click(screen.getByRole("button", { name: /just explore/i }));

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
    });
  });

  it("is a full-screen centered layout", () => {
    const { container } = render(<WelcomeScreen />);
    const root = container.firstChild as HTMLElement;
    expect(root).toHaveClass("h-screen", "w-screen");
  });
});
