import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { CreateWalletScreen } from "./CreateWalletScreen";

// Mock navigate
const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

// Mock bindings
vi.mock("@/bindings", () => ({
  commands: {
    walletCreate: vi.fn().mockResolvedValue({
      status: "ok",
      data: { seedHash: "abc123", alias: "Test Wallet" },
    }),
  },
}));

// Mock wallet store
const mockLoadWallets = vi.fn().mockResolvedValue(undefined);
vi.mock("@/stores/walletStore", () => ({
  useWalletStore: Object.assign(vi.fn().mockReturnValue({}), {
    getState: () => ({ loadWallets: mockLoadWallets }),
  }),
}));

// Mock sonner
vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
  },
}));

// Mock @scure/bip39
vi.mock("@scure/bip39", () => ({
  generateMnemonic: vi.fn(
    () =>
      "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
  ),
}));

vi.mock("@scure/bip39/wordlists/english.js", () => ({
  wordlist: ["abandon", "ability", "able", "about"],
}));

vi.mock("@scure/bip39/wordlists/spanish.js", () => ({
  wordlist: ["ábaco", "abdomen", "abeja", "abierto"],
}));

vi.mock("@scure/bip39/wordlists/french.js", () => ({
  wordlist: ["abaisser", "abandon", "abdiquer", "abeille"],
}));

vi.mock("@scure/bip39/wordlists/italian.js", () => ({
  wordlist: ["abaco", "abbaglio", "abbondante", "abete"],
}));

vi.mock("@scure/bip39/wordlists/portuguese.js", () => ({
  wordlist: ["abacate", "abaixo", "abalar", "abater"],
}));

import { commands } from "@/bindings";
import { generateMnemonic } from "@scure/bip39";

describe("CreateWalletScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ─── Initial Render ──────────────────────────────────────────

  it("renders the screen title", () => {
    render(<CreateWalletScreen />);
    expect(
      screen.getByRole("heading", { name: /create new wallet/i }),
    ).toBeInTheDocument();
  });

  it("renders back button", () => {
    render(<CreateWalletScreen />);
    expect(
      screen.getByRole("button", { name: /back to wallets/i }),
    ).toBeInTheDocument();
  });

  it("renders step indicator with Generate active", () => {
    render(<CreateWalletScreen />);
    expect(screen.getByText("Generate")).toBeInTheDocument();
    expect(screen.getByText("Backup")).toBeInTheDocument();
    expect(screen.getByText("Protect")).toBeInTheDocument();
  });

  it("navigates back when back button clicked", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(screen.getByRole("button", { name: /back to wallets/i }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  // ─── Step 1: Generate ────────────────────────────────────────

  it("renders Generate step by default", () => {
    render(<CreateWalletScreen />);
    expect(
      screen.getByRole("heading", { name: /generate seed phrase/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    ).toBeInTheDocument();
  });

  it("shows word count selector with default 24", () => {
    render(<CreateWalletScreen />);
    expect(screen.getByText("24 words")).toBeInTheDocument();
  });

  it("transitions to Backup step on Generate click", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    expect(
      screen.getByRole("heading", { name: /back up your seed phrase/i }),
    ).toBeInTheDocument();
  });

  // ─── Language Selection ───────────────────────────────────────

  it("shows language selector with default English", () => {
    render(<CreateWalletScreen />);
    expect(screen.getByText("English")).toBeInTheDocument();
  });

  it("renders Language label", () => {
    render(<CreateWalletScreen />);
    expect(screen.getByText("Language")).toBeInTheDocument();
  });

  it("passes selected language wordlist to generateMnemonic", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    // Default is English — click Generate
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    expect(generateMnemonic).toHaveBeenCalledWith(
      ["abandon", "ability", "able", "about"],
      expect.any(Number),
    );
  });

  // ─── Step 2: Backup ──────────────────────────────────────────

  it("displays generated words in the backup step", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    // The mock generates "abandon" words
    expect(screen.getAllByText("abandon").length).toBeGreaterThan(0);
  });

  it("shows word grid with list items", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    const items = screen.getAllByRole("listitem");
    expect(items.length).toBe(12); // "abandon...about" is 12 words
  });

  it("renders Copy All Words button", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    expect(screen.getByText(/copy all words/i)).toBeInTheDocument();
  });

  it("Continue button is disabled until checkbox checked", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    const continueBtn = screen.getByRole("button", { name: /continue/i });
    expect(continueBtn).toBeDisabled();
  });

  it("Continue button enables after checking checkbox", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    const checkbox = screen.getByRole("checkbox");
    await user.click(checkbox);
    const continueBtn = screen.getByRole("button", { name: /continue/i });
    expect(continueBtn).not.toBeDisabled();
  });

  it("transitions to Protect step after checking and clicking Continue", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    await user.click(screen.getByRole("checkbox"));
    await user.click(screen.getByRole("button", { name: /continue/i }));
    expect(
      screen.getByRole("heading", { name: /name & protect/i }),
    ).toBeInTheDocument();
  });

  it("going back from Backup clears mnemonic and returns to Generate", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    await user.click(screen.getByRole("button", { name: /^back$/i }));
    expect(
      screen.getByRole("heading", { name: /generate seed phrase/i }),
    ).toBeInTheDocument();
  });

  // ─── Step 3: Protect ─────────────────────────────────────────

  async function goToProtectStep() {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    await user.click(screen.getByRole("checkbox"));
    await user.click(screen.getByRole("button", { name: /continue/i }));
    return user;
  }

  it("renders wallet name input in Protect step", async () => {
    await goToProtectStep();
    expect(screen.getByLabelText(/wallet name/i)).toBeInTheDocument();
  });

  it("renders password input in Protect step", async () => {
    await goToProtectStep();
    expect(
      screen.getByLabelText(/password \(optional\)/i),
    ).toBeInTheDocument();
  });

  it("shows password strength meter when password entered", async () => {
    const user = await goToProtectStep();
    await user.type(
      screen.getByLabelText(/password \(optional\)/i),
      "MyP@ssword1",
    );
    expect(screen.getByText(/strength:/i)).toBeInTheDocument();
  });

  it("toggles password visibility", async () => {
    const user = await goToProtectStep();
    const passwordInput = screen.getByLabelText(
      /password \(optional\)/i,
    ) as HTMLInputElement;
    expect(passwordInput.type).toBe("password");
    await user.click(screen.getByRole("button", { name: /show password/i }));
    expect(passwordInput.type).toBe("text");
  });

  it("enforces 64 char limit on wallet name", async () => {
    const user = await goToProtectStep();
    const nameInput = screen.getByLabelText(/wallet name/i);
    const longName = "a".repeat(70);
    await user.type(nameInput, longName);
    expect((nameInput as HTMLInputElement).value.length).toBeLessThanOrEqual(64);
  });

  it("shows character counter when name exceeds 50 chars", async () => {
    const user = await goToProtectStep();
    const nameInput = screen.getByLabelText(/wallet name/i);
    await user.type(nameInput, "a".repeat(55));
    expect(screen.getByText(/55\/64/)).toBeInTheDocument();
  });

  it("renders Create Wallet button", async () => {
    await goToProtectStep();
    expect(
      screen.getByRole("button", { name: /create wallet/i }),
    ).toBeInTheDocument();
  });

  it("calls walletCreate on save", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(commands.walletCreate).toHaveBeenCalledWith({
        mnemonic: expect.any(String),
        password: "",
        alias: "",
        usePasswordForApp: false,
      });
    });
  });

  it("calls walletCreate with password and alias", async () => {
    const user = await goToProtectStep();
    await user.type(screen.getByLabelText(/wallet name/i), "Test Wallet");
    await user.type(
      screen.getByLabelText(/password \(optional\)/i),
      "pass123",
    );
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(commands.walletCreate).toHaveBeenCalledWith({
        mnemonic: expect.any(String),
        password: "pass123",
        alias: "Test Wallet",
        usePasswordForApp: true,
      });
    });
  });

  it("reloads wallet store after successful creation", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(mockLoadWallets).toHaveBeenCalled();
    });
  });

  // ─── Success Screen ──────────────────────────────────────────

  it("shows success screen after wallet creation", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: /wallet created successfully/i }),
      ).toBeInTheDocument();
    });
  });

  it("success screen has Go to Wallet button", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /go to wallet/i }),
      ).toBeInTheDocument();
    });
  });

  it("success screen has Create Identity button", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /create identity/i }),
      ).toBeInTheDocument();
    });
  });

  it("Go to Wallet navigates to /wallets", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /go to wallet/i }),
      ).toBeInTheDocument();
    });
    await user.click(screen.getByRole("button", { name: /go to wallet/i }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  it("Create Identity navigates to /identities", async () => {
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /create identity/i }),
      ).toBeInTheDocument();
    });
    await user.click(
      screen.getByRole("button", { name: /create identity/i }),
    );
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
  });

  // ─── Error Handling ──────────────────────────────────────────

  it("shows toast error on walletCreate failure", async () => {
    vi.mocked(commands.walletCreate).mockResolvedValueOnce({
      status: "error",
      error: "Wallet already exists",
    });
    const { toast } = await import("sonner");
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Wallet already exists");
    });
  });

  it("shows toast error on exception", async () => {
    vi.mocked(commands.walletCreate).mockRejectedValueOnce(
      new Error("Network error"),
    );
    const { toast } = await import("sonner");
    const user = await goToProtectStep();
    await user.click(screen.getByRole("button", { name: /create wallet/i }));
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Network error");
    });
  });

  // ─── Accessibility ───────────────────────────────────────────

  it("step indicator has navigation role", () => {
    render(<CreateWalletScreen />);
    expect(
      screen.getByRole("navigation", { name: /wallet creation steps/i }),
    ).toBeInTheDocument();
  });

  it("active step has aria-current", () => {
    render(<CreateWalletScreen />);
    const activeStep = screen.getByText("Generate").closest("[aria-current]");
    expect(activeStep).toHaveAttribute("aria-current", "step");
  });

  it("seed phrase list has list role", async () => {
    const user = userEvent.setup();
    render(<CreateWalletScreen />);
    await user.click(
      screen.getByRole("button", { name: /generate seed phrase/i }),
    );
    expect(
      screen.getByRole("list", { name: /seed phrase words/i }),
    ).toBeInTheDocument();
  });
});
