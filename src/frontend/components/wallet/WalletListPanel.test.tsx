import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { WalletListPanel } from "./WalletListPanel";
import {
  createMockHdWallet,
  createMockSingleKeyWallet,
} from "@/test/fixtures";
import type { WalletDto, SingleKeyWalletDto, WalletRefDto } from "@/bindings";

// ─── Local wrappers over centralized fixtures (test-specific defaults) ──

function makeHdWallet(overrides: Partial<WalletDto> = {}): WalletDto {
  return createMockHdWallet({
    seedHash: "abc123",
    usesPassword: false,
    alias: "My HD Wallet",
    isMain: true,
    confirmedBalance: 250000000, // 2.5 DASH
    unconfirmedBalance: 0,
    totalBalance: 250000000,
    addresses: [],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [],
    passwordHint: null,
    ...overrides,
  });
}

function makeSingleKeyWallet(
  overrides: Partial<SingleKeyWalletDto> = {},
): SingleKeyWalletDto {
  return createMockSingleKeyWallet({
    keyHash: "def456",
    usesPassword: false,
    publicKey: "02aabbcc",
    address: "XjBxRk1EUpAWBfiT2vEqdP5f4HdS1t5Xrz",
    alias: "My Single Key",
    confirmedBalance: 100000000, // 1.0 DASH
    unconfirmedBalance: 0,
    totalBalance: 100000000,
    utxoCount: 3,
    utxos: [],
    ...overrides,
  });
}

const defaultProps = {
  hdWallets: [] as WalletDto[],
  singleKeyWallets: [] as SingleKeyWalletDto[],
  selectedWallet: null as WalletRefDto | null,
  onSelectWallet: vi.fn(),
  onRenameHdWallet: vi.fn().mockResolvedValue(undefined),
  onRenameSingleKeyWallet: vi.fn().mockResolvedValue(undefined),
  onRemoveHdWallet: vi.fn().mockResolvedValue(undefined),
  onRemoveSingleKeyWallet: vi.fn().mockResolvedValue(undefined),
};

function setup(props: Partial<Parameters<typeof WalletListPanel>[0]> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(<WalletListPanel {...mergedProps} />),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Empty state ───────────────────────────────────────────────────

describe("WalletListPanel — empty state", () => {
  it("renders empty state when no wallets", () => {
    setup();
    expect(screen.getByText("No Wallets Loaded")).toBeInTheDocument();
    expect(
      screen.getByText("Create or import a wallet to get started."),
    ).toBeInTheDocument();
  });

  it("shows Create Wallet button when onCreateWallet provided", () => {
    const onCreateWallet = vi.fn();
    setup({ onCreateWallet });
    expect(
      screen.getByRole("button", { name: "Create Wallet" }),
    ).toBeInTheDocument();
  });

  it("calls onCreateWallet when Create Wallet clicked", async () => {
    const onCreateWallet = vi.fn();
    const { user } = setup({ onCreateWallet });
    await user.click(screen.getByRole("button", { name: "Create Wallet" }));
    expect(onCreateWallet).toHaveBeenCalledOnce();
  });

  it("shows Import Wallet button when onImportWallet provided", () => {
    const onImportWallet = vi.fn();
    setup({ onImportWallet });
    expect(
      screen.getByRole("button", { name: "Import Wallet" }),
    ).toBeInTheDocument();
  });

  it("calls onImportWallet when Import Wallet clicked", async () => {
    const onImportWallet = vi.fn();
    const { user } = setup({ onImportWallet });
    await user.click(screen.getByRole("button", { name: "Import Wallet" }));
    expect(onImportWallet).toHaveBeenCalledOnce();
  });

  it("has wallet list region role", () => {
    setup();
    expect(screen.getByRole("region", { name: "Wallet list" })).toBeInTheDocument();
  });
});

// ─── HD Wallet section ─────────────────────────────────────────────

describe("WalletListPanel — HD wallets", () => {
  const hdWallet = makeHdWallet();

  it("renders HD Wallets section header with count", () => {
    setup({ hdWallets: [hdWallet] });
    expect(screen.getByText("HD Wallets")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("renders wallet name and balance", () => {
    setup({ hdWallets: [hdWallet] });
    expect(screen.getByText("My HD Wallet")).toBeInTheDocument();
    expect(screen.getByText("2.50000000 DASH")).toBeInTheDocument();
  });

  it("shows 'Unnamed Wallet' when alias is null", () => {
    setup({ hdWallets: [makeHdWallet({ alias: null })] });
    expect(screen.getByText("Unnamed Wallet")).toBeInTheDocument();
  });

  it("shows pending badge when unconfirmed balance > 0", () => {
    setup({
      hdWallets: [
        makeHdWallet({
          unconfirmedBalance: 50000000,
          totalBalance: 300000000,
        }),
      ],
    });
    expect(screen.getByText("pending")).toBeInTheDocument();
  });

  it("does not show pending badge when no unconfirmed balance", () => {
    setup({ hdWallets: [hdWallet] });
    expect(screen.queryByText("pending")).not.toBeInTheDocument();
  });

  it("shows lock icon when wallet uses password", () => {
    setup({ hdWallets: [makeHdWallet({ usesPassword: true })] });
    expect(screen.getByLabelText("Password protected")).toBeInTheDocument();
  });

  it("does not show lock icon when wallet has no password", () => {
    setup({ hdWallets: [hdWallet] });
    expect(screen.queryByLabelText("Password protected")).not.toBeInTheDocument();
  });

  it("renders multiple HD wallets", () => {
    setup({
      hdWallets: [
        makeHdWallet({ seedHash: "a", alias: "Wallet A" }),
        makeHdWallet({ seedHash: "b", alias: "Wallet B" }),
      ],
    });
    expect(screen.getByText("Wallet A")).toBeInTheDocument();
    expect(screen.getByText("Wallet B")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument(); // count badge
  });

  it("highlights selected wallet", () => {
    const wallet = makeHdWallet({ seedHash: "sel" });
    setup({
      hdWallets: [wallet],
      selectedWallet: { type: "hd", seedHash: "sel" },
    });
    const button = screen.getByRole("button", { name: /My HD Wallet/i });
    expect(button).toHaveAttribute("aria-current", "true");
  });

  it("does not highlight unselected wallet", () => {
    setup({ hdWallets: [hdWallet] });
    const button = screen.getByRole("button", { name: /My HD Wallet/i });
    expect(button).not.toHaveAttribute("aria-current");
  });
});

// ─── Single-Key Wallet section ─────────────────────────────────────

describe("WalletListPanel — single-key wallets", () => {
  const skWallet = makeSingleKeyWallet();

  it("renders Single-Key Wallets section header with count", () => {
    setup({ singleKeyWallets: [skWallet] });
    expect(screen.getByText("Single-Key Wallets")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("renders wallet name and balance", () => {
    setup({ singleKeyWallets: [skWallet] });
    expect(screen.getByText("My Single Key")).toBeInTheDocument();
    expect(screen.getByText("1.00000000 DASH")).toBeInTheDocument();
  });

  it("shows pending badge when unconfirmed balance > 0", () => {
    setup({
      singleKeyWallets: [
        makeSingleKeyWallet({
          unconfirmedBalance: 10000000,
          totalBalance: 110000000,
        }),
      ],
    });
    expect(screen.getByText("pending")).toBeInTheDocument();
  });

  it("highlights selected single-key wallet", () => {
    const wallet = makeSingleKeyWallet({ keyHash: "sel" });
    setup({
      singleKeyWallets: [wallet],
      selectedWallet: { type: "singleKey", keyHash: "sel" },
    });
    const button = screen.getByRole("button", { name: /My Single Key/i });
    expect(button).toHaveAttribute("aria-current", "true");
  });
});

// ─── Selection ─────────────────────────────────────────────────────

describe("WalletListPanel — selection", () => {
  it("calls onSelectWallet with HD ref when clicked", async () => {
    const onSelectWallet = vi.fn();
    const { user } = setup({
      hdWallets: [makeHdWallet({ seedHash: "h1" })],
      onSelectWallet,
    });
    await user.click(screen.getByRole("button", { name: /My HD Wallet/i }));
    expect(onSelectWallet).toHaveBeenCalledWith({
      type: "hd",
      seedHash: "h1",
    });
  });

  it("calls onSelectWallet with single-key ref when clicked", async () => {
    const onSelectWallet = vi.fn();
    const { user } = setup({
      singleKeyWallets: [makeSingleKeyWallet({ keyHash: "k1" })],
      onSelectWallet,
    });
    await user.click(screen.getByRole("button", { name: /My Single Key/i }));
    expect(onSelectWallet).toHaveBeenCalledWith({
      type: "singleKey",
      keyHash: "k1",
    });
  });
});

// ─── Context menu — Rename ─────────────────────────────────────────

/**
 * Helper to trigger a Radix DropdownMenu item action.
 * Open the menu trigger, then click the menu item text.
 */
async function clickMenuItem(
  user: ReturnType<typeof userEvent.setup>,
  triggerLabel: string,
  itemText: string,
) {
  await user.click(screen.getByLabelText(triggerLabel));
  await user.click(screen.getByText(itemText));
}

describe("WalletListPanel — rename", () => {
  it("shows inline rename input on HD wallet menu Rename click", async () => {
    const { user } = setup({
      hdWallets: [makeHdWallet()],
    });
    await clickMenuItem(user, "Wallet actions", "Rename");
    const input = await screen.findByRole("textbox", { name: "Wallet name" });
    expect(input).toBeInTheDocument();
  });

  it("calls onRenameHdWallet when Enter is pressed on rename input", async () => {
    const onRenameHdWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [makeHdWallet({ seedHash: "r1" })],
      onRenameHdWallet,
    });
    await clickMenuItem(user, "Wallet actions", "Rename");
    const input = await screen.findByRole("textbox", { name: "Wallet name" });
    await user.clear(input);
    await user.type(input, "New Name{Enter}");
    expect(onRenameHdWallet).toHaveBeenCalledWith("r1", "New Name");
  });

  it("cancels rename on Escape", async () => {
    const onRenameHdWallet = vi.fn();
    const { user } = setup({
      hdWallets: [makeHdWallet()],
      onRenameHdWallet,
    });
    await clickMenuItem(user, "Wallet actions", "Rename");
    const input = await screen.findByRole("textbox", { name: "Wallet name" });
    await user.type(input, "{Escape}");
    await waitFor(() => {
      expect(screen.queryByRole("textbox", { name: "Wallet name" })).not.toBeInTheDocument();
    });
    expect(onRenameHdWallet).not.toHaveBeenCalled();
  });

  it("calls onRenameSingleKeyWallet on rename", async () => {
    const onRenameSingleKeyWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      singleKeyWallets: [makeSingleKeyWallet({ keyHash: "sk1" })],
      onRenameSingleKeyWallet,
    });
    await clickMenuItem(user, "Wallet actions", "Rename");
    const input = await screen.findByRole("textbox", { name: "Wallet name" });
    await user.clear(input);
    await user.type(input, "SK Renamed{Enter}");
    expect(onRenameSingleKeyWallet).toHaveBeenCalledWith("sk1", "SK Renamed");
  });

  it("saves null alias when rename input is empty", async () => {
    const onRenameHdWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [makeHdWallet({ seedHash: "e1" })],
      onRenameHdWallet,
    });
    await clickMenuItem(user, "Wallet actions", "Rename");
    const input = await screen.findByRole("textbox", { name: "Wallet name" });
    await user.clear(input);
    await user.type(input, "{Enter}");
    expect(onRenameHdWallet).toHaveBeenCalledWith("e1", null);
  });
});

// ─── Context menu — Remove ─────────────────────────────────────────

describe("WalletListPanel — remove", () => {
  it("shows confirmation dialog on HD wallet Remove click", async () => {
    const { user } = setup({
      hdWallets: [makeHdWallet({ alias: "Deletable" })],
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Remove"));
    expect(screen.getByText("Remove Wallet")).toBeInTheDocument();
    expect(
      screen.getByText(
        'Are you sure you want to remove "Deletable"? This action cannot be undone.',
      ),
    ).toBeInTheDocument();
  });

  it("calls onRemoveHdWallet when confirmed", async () => {
    const onRemoveHdWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [makeHdWallet({ seedHash: "del1" })],
      onRemoveHdWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Remove"));
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect(onRemoveHdWallet).toHaveBeenCalledWith("del1");
  });

  it("does not call onRemoveHdWallet when canceled", async () => {
    const onRemoveHdWallet = vi.fn();
    const { user } = setup({
      hdWallets: [makeHdWallet()],
      onRemoveHdWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Remove"));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onRemoveHdWallet).not.toHaveBeenCalled();
  });

  it("shows confirmation dialog on single-key wallet Remove click", async () => {
    const { user } = setup({
      singleKeyWallets: [makeSingleKeyWallet({ alias: "SK Delete" })],
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Remove"));
    expect(screen.getByText("Remove Wallet")).toBeInTheDocument();
    expect(
      screen.getByText(
        'Are you sure you want to remove "SK Delete"? This action cannot be undone.',
      ),
    ).toBeInTheDocument();
  });

  it("calls onRemoveSingleKeyWallet when confirmed", async () => {
    const onRemoveSingleKeyWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      singleKeyWallets: [makeSingleKeyWallet({ keyHash: "skdel" })],
      onRemoveSingleKeyWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Remove"));
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect(onRemoveSingleKeyWallet).toHaveBeenCalledWith("skdel");
  });
});

// ─── Context menu — Lock/Unlock ────────────────────────────────────

describe("WalletListPanel — lock/unlock", () => {
  it("shows Unlock menu item for password-protected HD wallet", async () => {
    const onUnlockWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [makeHdWallet({ usesPassword: true })],
      onUnlockWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    expect(screen.getByText("Unlock")).toBeInTheDocument();
  });

  it("shows Lock menu item for password-protected HD wallet", async () => {
    const onLockWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [makeHdWallet({ usesPassword: true })],
      onLockWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    expect(screen.getByText("Lock")).toBeInTheDocument();
  });

  it("does not show Unlock/Lock for wallets without password", async () => {
    const { user } = setup({
      hdWallets: [makeHdWallet({ usesPassword: false })],
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    expect(screen.queryByText("Unlock")).not.toBeInTheDocument();
    expect(screen.queryByText("Lock")).not.toBeInTheDocument();
  });

  it("opens wallet unlock dialog on Unlock click", async () => {
    const onUnlockWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [
        makeHdWallet({
          usesPassword: true,
          alias: "Locked Wallet",
          passwordHint: "hint123",
        }),
      ],
      onUnlockWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Unlock"));
    // Unlock dialog should appear
    expect(screen.getByText(/Unlock Wallet/i)).toBeInTheDocument();
  });

  it("calls onLockWallet on Lock click", async () => {
    const onLockWallet = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      hdWallets: [makeHdWallet({ usesPassword: true, seedHash: "lock1" })],
      onLockWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Lock"));
    expect(onLockWallet).toHaveBeenCalledWith("lock1");
  });

  it("passes password to onUnlockWallet when unlocking", async () => {
    const onUnlockWallet = vi.fn().mockResolvedValue(null);
    const { user } = setup({
      hdWallets: [
        makeHdWallet({
          usesPassword: true,
          seedHash: "pw1",
          alias: "PW Wallet",
        }),
      ],
      onUnlockWallet,
    });
    // Open menu and click Unlock
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Unlock"));
    // Type password in the dialog and submit
    const input = await screen.findByPlaceholderText("Enter password");
    await user.type(input, "mysecret");
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    await waitFor(() => {
      expect(onUnlockWallet).toHaveBeenCalledWith("pw1", "mysecret");
    });
  });

  it("keeps dialog open and shows error when unlock fails", async () => {
    const onUnlockWallet = vi
      .fn()
      .mockResolvedValue("Incorrect password");
    const { user } = setup({
      hdWallets: [
        makeHdWallet({
          usesPassword: true,
          seedHash: "fail1",
          alias: "Fail Wallet",
        }),
      ],
      onUnlockWallet,
    });
    await user.click(screen.getByLabelText("Wallet actions"));
    await user.click(screen.getByText("Unlock"));
    const input = await screen.findByPlaceholderText("Enter password");
    await user.type(input, "wrongpw");
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    // Dialog should stay open with error
    await waitFor(() => {
      expect(screen.getByText("Incorrect password")).toBeInTheDocument();
    });
    // Dialog is still open (password field still visible)
    expect(screen.getByPlaceholderText("Enter password")).toBeInTheDocument();
  });
});

// ─── Mixed wallets ─────────────────────────────────────────────────

describe("WalletListPanel — mixed wallets", () => {
  it("renders both sections when both types exist", () => {
    setup({
      hdWallets: [makeHdWallet()],
      singleKeyWallets: [makeSingleKeyWallet()],
    });
    expect(screen.getByText("HD Wallets")).toBeInTheDocument();
    expect(screen.getByText("Single-Key Wallets")).toBeInTheDocument();
  });

  it("does not render HD section when only single-key wallets exist", () => {
    setup({
      singleKeyWallets: [makeSingleKeyWallet()],
    });
    expect(screen.queryByText("HD Wallets")).not.toBeInTheDocument();
    expect(screen.getByText("Single-Key Wallets")).toBeInTheDocument();
  });

  it("does not render single-key section when only HD wallets exist", () => {
    setup({
      hdWallets: [makeHdWallet()],
    });
    expect(screen.getByText("HD Wallets")).toBeInTheDocument();
    expect(screen.queryByText("Single-Key Wallets")).not.toBeInTheDocument();
  });

  it("has correct listbox roles", () => {
    setup({
      hdWallets: [makeHdWallet()],
      singleKeyWallets: [makeSingleKeyWallet()],
    });
    expect(screen.getByRole("listbox", { name: "HD wallets" })).toBeInTheDocument();
    expect(screen.getByRole("listbox", { name: "Single-key wallets" })).toBeInTheDocument();
  });
});

// ─── Balance formatting ────────────────────────────────────────────

describe("WalletListPanel — balance display", () => {
  it("formats zero balance correctly", () => {
    setup({
      hdWallets: [makeHdWallet({ totalBalance: 0 })],
    });
    expect(screen.getByText("0.00000000 DASH")).toBeInTheDocument();
  });

  it("formats large balance correctly", () => {
    setup({
      hdWallets: [makeHdWallet({ totalBalance: 1234567890 })],
    });
    expect(screen.getByText("12.34567890 DASH")).toBeInTheDocument();
  });

  it("formats small balance correctly", () => {
    setup({
      singleKeyWallets: [makeSingleKeyWallet({ totalBalance: 1 })],
    });
    expect(screen.getByText("0.00000001 DASH")).toBeInTheDocument();
  });
});

// ─── Accessibility ─────────────────────────────────────────────────

describe("WalletListPanel — accessibility", () => {
  it("has wallet list region with label", () => {
    setup({ hdWallets: [makeHdWallet()] });
    expect(
      screen.getByRole("region", { name: "Wallet list" }),
    ).toBeInTheDocument();
  });

  it("wallet action buttons have aria labels", () => {
    setup({ hdWallets: [makeHdWallet()] });
    expect(screen.getByLabelText("Wallet actions")).toBeInTheDocument();
  });

  it("accepts custom className", () => {
    const { container } = render(
      <WalletListPanel {...defaultProps} className="custom-class" />,
    );
    expect(container.firstChild).toHaveClass("custom-class");
  });
});
