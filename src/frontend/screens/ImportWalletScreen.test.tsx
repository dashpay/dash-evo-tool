import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { ImportWalletScreen } from "./ImportWalletScreen";

// Mock navigate
const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

// Mock bindings
vi.mock("@/bindings", () => ({
  commands: {
    walletImportMnemonic: vi.fn().mockResolvedValue({
      status: "ok",
      data: { seedHash: "abc123", alias: "Imported Wallet" },
    }),
    walletImportPrivateKey: vi.fn().mockResolvedValue({
      status: "ok",
      data: { keyHash: "def456", alias: "Imported Key", address: "Xabc..." },
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

// Mock toastError
const mockToastError = vi.fn();
vi.mock("@/lib/toastError", () => ({
  toastError: (...args: unknown[]) => mockToastError(...args),
}));

// Valid 24-word mnemonic for testing (matches the default 24-word form)
const VALID_WORDS = [
  "abandon", "abandon", "abandon", "abandon", "abandon", "abandon",
  "abandon", "abandon", "abandon", "abandon", "abandon", "abandon",
  "abandon", "abandon", "abandon", "abandon", "abandon", "abandon",
  "abandon", "abandon", "abandon", "abandon", "abandon", "about",
];
const VALID_MNEMONIC = VALID_WORDS.join(" ");

vi.mock("@scure/bip39", () => ({
  validateMnemonic: vi.fn((phrase: string) => {
    return phrase === VALID_MNEMONIC;
  }),
}));

vi.mock("@scure/bip39/wordlists/english.js", () => ({
  wordlist: [
    "abandon",
    "ability",
    "able",
    "about",
    "above",
    "absent",
    "absorb",
    "abstract",
    "absurd",
    "abuse",
    "access",
    "accident",
  ],
}));

import { commands } from "@/bindings";

/**
 * Helper: fill all word inputs individually by firing change events.
 */
function fillWordsDirectly(words: string[]) {
  const inputs = screen.getAllByRole("textbox");
  for (let i = 0; i < words.length && i < inputs.length; i++) {
    fireEvent.change(inputs[i], { target: { value: words[i] } });
  }
}

describe("ImportWalletScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ─── Initial Render ──────────────────────────────────────────

  it("renders the screen title", () => {
    render(<ImportWalletScreen />);
    expect(
      screen.getByRole("heading", { name: /import wallet/i }),
    ).toBeInTheDocument();
  });

  it("renders back button", () => {
    render(<ImportWalletScreen />);
    expect(
      screen.getByRole("button", { name: /back to wallets/i }),
    ).toBeInTheDocument();
  });

  it("renders Seed Phrase and Private Key tabs", () => {
    render(<ImportWalletScreen />);
    expect(
      screen.getByRole("tab", { name: /seed phrase/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: /private key/i }),
    ).toBeInTheDocument();
  });

  it("Seed Phrase tab is active by default", () => {
    render(<ImportWalletScreen />);
    expect(screen.getByRole("tab", { name: /seed phrase/i })).toHaveAttribute(
      "data-state",
      "active",
    );
  });

  it("navigates back when back button clicked", async () => {
    const user = userEvent.setup();
    render(<ImportWalletScreen />);
    await user.click(screen.getByRole("button", { name: /back to wallets/i }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  // ─── Mnemonic Import Tab ─────────────────────────────────────

  it("renders mnemonic form heading", () => {
    render(<ImportWalletScreen />);
    expect(
      screen.getByRole("heading", { name: /import from seed phrase/i }),
    ).toBeInTheDocument();
  });

  it("renders word count selector defaulting to 24", () => {
    render(<ImportWalletScreen />);
    expect(screen.getByText("24 words")).toBeInTheDocument();
  });

  it("renders 24 word input fields", () => {
    render(<ImportWalletScreen />);
    expect(screen.getByLabelText("Word 1")).toBeInTheDocument();
    expect(screen.getByLabelText("Word 24")).toBeInTheDocument();
  });

  it("does not show name/password section when mnemonic incomplete", () => {
    render(<ImportWalletScreen />);
    expect(screen.queryByLabelText(/^name$/i)).not.toBeInTheDocument();
  });

  it("shows validation error for invalid checksum", async () => {
    render(<ImportWalletScreen />);
    // Fill all 24 words with "abandon" — valid word but bad checksum
    const inputs = screen.getAllByRole("textbox");
    for (let i = 0; i < 24; i++) {
      fireEvent.change(inputs[i], { target: { value: "abandon" } });
    }
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(
      screen.getByText(/checksum verification failed/i),
    ).toBeInTheDocument();
  });

  it("shows error for unknown word", async () => {
    render(<ImportWalletScreen />);
    const inputs = screen.getAllByRole("textbox");
    // Fill all with "abandon" except put an invalid word at position 3
    for (let i = 0; i < 24; i++) {
      fireEvent.change(inputs[i], {
        target: { value: i === 2 ? "zzzzz" : "abandon" },
      });
    }
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText(/word 3/i)).toBeInTheDocument();
  });

  it("handles multi-word paste into first field", () => {
    render(<ImportWalletScreen />);
    const firstInput = screen.getByLabelText("Word 1");
    // Simulate pasting multiple words via onChange
    fireEvent.change(firstInput, {
      target: { value: "abandon abandon abandon" },
    });
    const inputs = screen.getAllByRole("textbox") as HTMLInputElement[];
    expect(inputs[0].value).toBe("abandon");
    expect(inputs[1].value).toBe("abandon");
    expect(inputs[2].value).toBe("abandon");
  });

  it("shows advanced options when toggled", async () => {
    const user = userEvent.setup();
    render(<ImportWalletScreen />);
    await user.click(screen.getByLabelText(/show advanced options/i));
    expect(
      screen.getByLabelText(/identity auto-discovery/i),
    ).toBeInTheDocument();
  });

  it("identity scan count defaults to 10", async () => {
    const user = userEvent.setup();
    render(<ImportWalletScreen />);
    await user.click(screen.getByLabelText(/show advanced options/i));
    const scanInput = screen.getByLabelText(
      /identity auto-discovery/i,
    ) as HTMLInputElement;
    expect(scanInput.value).toBe("10");
  });

  // ─── Mnemonic Import: Valid Form ─────────────────────────────

  function renderAndFillValidMnemonic() {
    render(<ImportWalletScreen />);
    fillWordsDirectly(VALID_WORDS);
  }

  it("shows name/password when valid mnemonic entered", () => {
    renderAndFillValidMnemonic();
    expect(screen.getByLabelText(/^name$/i)).toBeInTheDocument();
  });

  it("shows Import Wallet button when valid", () => {
    renderAndFillValidMnemonic();
    expect(
      screen.getByRole("button", { name: /import wallet/i }),
    ).toBeInTheDocument();
  });

  it("calls walletImportMnemonic on save", async () => {
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(commands.walletImportMnemonic).toHaveBeenCalledWith({
        mnemonic: VALID_MNEMONIC,
        password: "",
        alias: "",
        usePasswordForApp: false,
        identityScanCount: 10,
      });
    });
  });

  it("reloads wallet store after successful import", async () => {
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(mockLoadWallets).toHaveBeenCalled();
    });
  });

  it("shows success screen after mnemonic import", async () => {
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(
        screen.getByRole("heading", {
          name: /wallet imported successfully/i,
        }),
      ).toBeInTheDocument();
    });
  });

  it("HD success screen shows Create Identity button", async () => {
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /create identity/i }),
      ).toBeInTheDocument();
    });
  });

  it("HD success screen shows Import Another button", async () => {
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /import another/i }),
      ).toBeInTheDocument();
    });
  });

  it("shows toast on mnemonic import error", async () => {
    vi.mocked(commands.walletImportMnemonic).mockResolvedValueOnce({
      status: "error",
      error: "Wallet already imported",
    });
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Wallet already imported");
    });
  });

  it("Import Another resets to import form", async () => {
    const user = userEvent.setup();
    renderAndFillValidMnemonic();
    await user.click(
      screen.getByRole("button", { name: /import wallet/i }),
    );
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /import another/i }),
      ).toBeInTheDocument();
    });
    await user.click(
      screen.getByRole("button", { name: /import another/i }),
    );
    expect(
      screen.getByRole("heading", { name: /import wallet/i }),
    ).toBeInTheDocument();
  });

  // ─── Private Key Tab ─────────────────────────────────────────

  async function switchToPrivateKeyTab() {
    const user = userEvent.setup();
    render(<ImportWalletScreen />);
    await user.click(screen.getByRole("tab", { name: /private key/i }));
    return user;
  }

  it("shows private key form when tab clicked", async () => {
    await switchToPrivateKeyTab();
    expect(
      screen.getByRole("heading", { name: /import private key/i }),
    ).toBeInTheDocument();
  });

  it("renders private key input field", async () => {
    await switchToPrivateKeyTab();
    expect(screen.getByPlaceholderText(/enter private key/i)).toBeInTheDocument();
  });

  it("private key input is password type by default", async () => {
    await switchToPrivateKeyTab();
    const input = screen.getByPlaceholderText(
      /enter private key/i,
    ) as HTMLInputElement;
    expect(input.type).toBe("password");
  });

  it("toggles private key visibility", async () => {
    const user = await switchToPrivateKeyTab();
    const input = screen.getByPlaceholderText(
      /enter private key/i,
    ) as HTMLInputElement;
    expect(input.type).toBe("password");
    await user.click(screen.getByRole("button", { name: /show key/i }));
    expect(input.type).toBe("text");
  });

  it("shows error for invalid key format", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "invalidkey",
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText(/invalid format/i)).toBeInTheDocument();
  });

  it("shows format detected for valid WIF-length key", async () => {
    const user = await switchToPrivateKeyTab();
    const wif = "5" + "K".repeat(50);
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      wif,
    );
    expect(screen.getByText(/wif format detected/i)).toBeInTheDocument();
  });

  it("shows format detected for valid hex-length key", async () => {
    const user = await switchToPrivateKeyTab();
    const hex = "a".repeat(64);
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      hex,
    );
    expect(screen.getByText(/hex format detected/i)).toBeInTheDocument();
  });

  it("does not show name/password when no key entered", async () => {
    await switchToPrivateKeyTab();
    expect(screen.queryByLabelText(/^name$/i)).not.toBeInTheDocument();
  });

  it("shows name/password when valid key entered", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    expect(screen.getByLabelText(/^name$/i)).toBeInTheDocument();
  });

  it("shows Import Key button when valid key entered", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    expect(
      screen.getByRole("button", { name: /import key/i }),
    ).toBeInTheDocument();
  });

  it("calls walletImportPrivateKey on save", async () => {
    const user = await switchToPrivateKeyTab();
    const hex = "a".repeat(64);
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      hex,
    );
    await user.click(screen.getByRole("button", { name: /import key/i }));
    await waitFor(() => {
      expect(commands.walletImportPrivateKey).toHaveBeenCalledWith({
        privateKey: hex,
        password: "",
        alias: "",
      });
    });
  });

  it("shows success screen after private key import", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    await user.click(screen.getByRole("button", { name: /import key/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: /key imported successfully/i }),
      ).toBeInTheDocument();
    });
  });

  it("private key success screen does NOT show Create Identity", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    await user.click(screen.getByRole("button", { name: /import key/i }));
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: /key imported successfully/i }),
      ).toBeInTheDocument();
    });
    expect(
      screen.queryByRole("button", { name: /create identity/i }),
    ).not.toBeInTheDocument();
  });

  it("shows toast on private key import error", async () => {
    vi.mocked(commands.walletImportPrivateKey).mockResolvedValueOnce({
      status: "error",
      error: "Key already imported",
    });
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    await user.click(screen.getByRole("button", { name: /import key/i }));
    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Key already imported");
    });
  });

  // ─── Password in Name/Password Section ───────────────────────

  it("shows password strength meter", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    await user.type(
      screen.getByLabelText(/password \(optional\)/i),
      "StrongP@ss1",
    );
    expect(screen.getByText(/strength:/i)).toBeInTheDocument();
  });

  it("password show/hide works in name section", async () => {
    const user = await switchToPrivateKeyTab();
    await user.type(
      screen.getByPlaceholderText(/enter private key/i),
      "a".repeat(64),
    );
    const passwordInput = screen.getByLabelText(
      /password \(optional\)/i,
    ) as HTMLInputElement;
    expect(passwordInput.type).toBe("password");
    await user.click(
      screen.getByRole("button", { name: /show password/i }),
    );
    expect(passwordInput.type).toBe("text");
  });

  // ─── Accessibility ───────────────────────────────────────────

  it("seed phrase input grid has group role", () => {
    render(<ImportWalletScreen />);
    expect(
      screen.getByRole("group", { name: /seed phrase input/i }),
    ).toBeInTheDocument();
  });

  it("tab list has tablist role", () => {
    render(<ImportWalletScreen />);
    expect(screen.getByRole("tablist")).toBeInTheDocument();
  });
});
