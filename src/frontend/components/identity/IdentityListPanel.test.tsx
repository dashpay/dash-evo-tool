import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { IdentityListPanel } from "./IdentityListPanel";
import type { QualifiedIdentityDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aabbccdd11223344",
    identityType: "user",
    alias: "Alice",
    balance: 250_000_000,
    keys: [
      {
        keyId: 0,
        keyType: "ECDSA_SECP256K1",
        purpose: "AUTHENTICATION",
        securityLevel: "MASTER",
        data: "02aabb",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed123"],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

const defaultProps = {
  identities: [] as QualifiedIdentityDto[],
  selectedIdentityId: null as string | null,
  refreshingIds: new Set<string>(),
  refreshingAll: false,
  onSelectIdentity: vi.fn(),
  onSetAlias: vi.fn().mockResolvedValue(undefined),
  onReorder: vi.fn().mockResolvedValue(undefined),
  onRemoveIdentity: vi.fn().mockResolvedValue(undefined),
  onRefreshIdentity: vi.fn().mockResolvedValue(undefined),
  onRefreshAll: vi.fn().mockResolvedValue(undefined),
};

function setup(
  props: Partial<Parameters<typeof IdentityListPanel>[0]> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <IdentityListPanel {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Empty state ───────────────────────────────────────────────────

describe("IdentityListPanel — empty state", () => {
  it("renders empty state when no identities", () => {
    setup();
    expect(screen.getByText("No Identities Loaded")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Load an existing identity or create a new one after setting up a wallet.",
      ),
    ).toBeInTheDocument();
  });

  it("shows Load Identity button when onLoadIdentity provided", () => {
    const onLoadIdentity = vi.fn();
    setup({ onLoadIdentity });
    expect(
      screen.getByRole("button", { name: "Load Identity" }),
    ).toBeInTheDocument();
  });

  it("calls onLoadIdentity when Load Identity clicked", async () => {
    const onLoadIdentity = vi.fn();
    const { user } = setup({ onLoadIdentity });
    await user.click(screen.getByRole("button", { name: "Load Identity" }));
    expect(onLoadIdentity).toHaveBeenCalledOnce();
  });

  it("shows Create Identity button when onCreateIdentity provided", () => {
    const onCreateIdentity = vi.fn();
    setup({ onCreateIdentity });
    expect(
      screen.getByRole("button", { name: /Create Identity/i }),
    ).toBeInTheDocument();
  });

  it("calls onCreateIdentity when Create Identity clicked", async () => {
    const onCreateIdentity = vi.fn();
    const { user } = setup({ onCreateIdentity });
    await user.click(
      screen.getByRole("button", { name: /Create Identity/i }),
    );
    expect(onCreateIdentity).toHaveBeenCalledOnce();
  });

  it("has identity list region role", () => {
    setup();
    expect(
      screen.getByRole("region", { name: "Identity list" }),
    ).toBeInTheDocument();
  });
});

// ─── Identity display ──────────────────────────────────────────────

describe("IdentityListPanel — identity display", () => {
  const alice = makeIdentity();

  it("renders identity alias and balance", () => {
    setup({ identities: [alice] });
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("2.50000000 DASH")).toBeInTheDocument();
  });

  it("shows truncated ID when no alias", () => {
    const noAlias = makeIdentity({ alias: null, id: "aabbccdd11223344" });
    setup({ identities: [noAlias] });
    expect(screen.getByText("aabbcc...223344")).toBeInTheDocument();
  });

  it("shows identity type badge", () => {
    setup({ identities: [alice] });
    expect(screen.getByText("User")).toBeInTheDocument();
  });

  it("shows Masternode type badge", () => {
    setup({
      identities: [makeIdentity({ identityType: "masternode" })],
    });
    expect(screen.getByText("Masternode")).toBeInTheDocument();
  });

  it("shows Evonode type badge", () => {
    setup({
      identities: [makeIdentity({ identityType: "evonode" })],
    });
    expect(screen.getByText("Evonode")).toBeInTheDocument();
  });

  it("shows status badge for non-active identities", () => {
    setup({
      identities: [makeIdentity({ status: "pendingCreation" })],
    });
    expect(screen.getByText("Pending")).toBeInTheDocument();
  });

  it("shows Failed status badge", () => {
    setup({
      identities: [makeIdentity({ status: "failedCreation" })],
    });
    expect(screen.getByText("Failed")).toBeInTheDocument();
  });

  it("does not show status badge for active identities", () => {
    setup({ identities: [alice] });
    expect(screen.queryByText("Active")).not.toBeInTheDocument();
  });

  it("shows identity count badge", () => {
    setup({
      identities: [
        makeIdentity({ id: "a1" }),
        makeIdentity({ id: "b2", alias: "Bob" }),
      ],
    });
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("shows private key indicator when keys have private keys", () => {
    setup({ identities: [alice] });
    expect(screen.getByLabelText("Has private keys")).toBeInTheDocument();
  });

  it("does not show private key indicator when no private keys", () => {
    const noPrivate = makeIdentity({
      keys: [
        {
          keyId: 0,
          keyType: "ECDSA_SECP256K1",
          purpose: "AUTHENTICATION",
          securityLevel: "MASTER",
          data: "02aabb",
          isDisabled: false,
          disabledAt: null,
          hasPrivateKey: false,
        },
      ],
    });
    setup({ identities: [noPrivate] });
    expect(
      screen.queryByLabelText("Has private keys"),
    ).not.toBeInTheDocument();
  });

  it("shows refreshing spinner for identity being refreshed", () => {
    setup({
      identities: [alice],
      refreshingIds: new Set(["aabbccdd11223344"]),
    });
    expect(screen.getByLabelText("Refreshing")).toBeInTheDocument();
  });

  it("shows copy button for identity ID", () => {
    setup({ identities: [alice] });
    expect(
      screen.getByLabelText("Copy to clipboard"),
    ).toBeInTheDocument();
  });

  it("renders multiple identities", () => {
    setup({
      identities: [
        makeIdentity({ id: "a1", alias: "Alice" }),
        makeIdentity({ id: "b2", alias: "Bob" }),
        makeIdentity({ id: "c3", alias: "Charlie" }),
      ],
    });
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("Bob")).toBeInTheDocument();
    expect(screen.getByText("Charlie")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });
});

// ─── Wallet association display ──────────────────────────────────────

describe("IdentityListPanel — wallet name display", () => {
  const walletNames: Record<string, string> = {
    seed123: "My HD Wallet",
    key456: "My Single Key",
  };

  it("shows wallet name when walletNames provided and identity has associated wallet", () => {
    setup({
      identities: [makeIdentity({ associatedWalletHashes: ["seed123"] })],
      walletNames,
    });
    expect(screen.getByTestId("wallet-name")).toHaveTextContent("My HD Wallet");
  });

  it("does not show wallet info when walletNames not provided", () => {
    setup({
      identities: [makeIdentity({ associatedWalletHashes: ["seed123"] })],
    });
    expect(screen.queryByTestId("wallet-name")).not.toBeInTheDocument();
  });

  it("does not show wallet info when identity has no associated wallets", () => {
    setup({
      identities: [makeIdentity({ associatedWalletHashes: [] })],
      walletNames,
    });
    expect(screen.queryByTestId("wallet-name")).not.toBeInTheDocument();
  });

  it("does not show wallet info when wallet hash not found in walletNames", () => {
    setup({
      identities: [makeIdentity({ associatedWalletHashes: ["unknown_hash"] })],
      walletNames,
    });
    expect(screen.queryByTestId("wallet-name")).not.toBeInTheDocument();
  });

  it("shows first matched wallet name when identity has multiple wallet hashes", () => {
    setup({
      identities: [
        makeIdentity({ associatedWalletHashes: ["seed123", "key456"] }),
      ],
      walletNames,
    });
    expect(screen.getByTestId("wallet-name")).toHaveTextContent("My HD Wallet");
  });

  it("falls through to second hash if first is not in walletNames", () => {
    setup({
      identities: [
        makeIdentity({ associatedWalletHashes: ["unknown", "key456"] }),
      ],
      walletNames,
    });
    expect(screen.getByTestId("wallet-name")).toHaveTextContent("My Single Key");
  });
});

// ─── Selection ─────────────────────────────────────────────────────

describe("IdentityListPanel — selection", () => {
  it("calls onSelectIdentity when identity is clicked", async () => {
    const onSelectIdentity = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ id: "sel1" })],
      onSelectIdentity,
    });
    await user.click(screen.getByText("Alice"));
    expect(onSelectIdentity).toHaveBeenCalledWith("sel1");
  });

  it("highlights selected identity", () => {
    setup({
      identities: [makeIdentity({ id: "sel1" })],
      selectedIdentityId: "sel1",
    });
    const option = screen.getByRole("option", { name: /Alice/i });
    expect(option).toHaveAttribute("aria-selected", "true");
  });

  it("does not highlight unselected identity", () => {
    setup({
      identities: [makeIdentity({ id: "sel1" })],
      selectedIdentityId: null,
    });
    const option = screen.getByRole("option", { name: /Alice/i });
    expect(option).toHaveAttribute("aria-selected", "false");
  });
});

// ─── Context menu — Update Alias ──────────────────────────────────

async function clickMenuItem(
  user: ReturnType<typeof userEvent.setup>,
  triggerLabel: string,
  itemText: string,
) {
  await user.click(screen.getByLabelText(triggerLabel));
  await user.click(screen.getByText(itemText));
}

describe("IdentityListPanel — alias editing", () => {
  it("shows inline alias editor on Update Alias click", async () => {
    const { user } = setup({ identities: [makeIdentity()] });
    await clickMenuItem(user, "Identity actions", "Update Alias");
    const input = await screen.findByRole("textbox", {
      name: "Identity alias",
    });
    expect(input).toBeInTheDocument();
  });

  it("calls onSetAlias when Enter pressed on alias input", async () => {
    const onSetAlias = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      identities: [makeIdentity({ id: "alias1" })],
      onSetAlias,
    });
    await clickMenuItem(user, "Identity actions", "Update Alias");
    const input = await screen.findByRole("textbox", {
      name: "Identity alias",
    });
    await user.clear(input);
    await user.type(input, "New Alias{Enter}");
    expect(onSetAlias).toHaveBeenCalledWith("alias1", "New Alias");
  });

  it("cancels alias edit on Escape", async () => {
    const onSetAlias = vi.fn();
    const { user } = setup({ identities: [makeIdentity()] });
    await clickMenuItem(user, "Identity actions", "Update Alias");
    const input = await screen.findByRole("textbox", {
      name: "Identity alias",
    });
    await user.type(input, "{Escape}");
    await waitFor(() => {
      expect(
        screen.queryByRole("textbox", { name: "Identity alias" }),
      ).not.toBeInTheDocument();
    });
    expect(onSetAlias).not.toHaveBeenCalled();
  });

  it("saves null alias when input is cleared", async () => {
    const onSetAlias = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      identities: [makeIdentity({ id: "e1" })],
      onSetAlias,
    });
    await clickMenuItem(user, "Identity actions", "Update Alias");
    const input = await screen.findByRole("textbox", {
      name: "Identity alias",
    });
    await user.clear(input);
    await user.type(input, "{Enter}");
    expect(onSetAlias).toHaveBeenCalledWith("e1", null);
  });

  it("shows set alias pencil button when identity has no alias", () => {
    setup({
      identities: [makeIdentity({ alias: null })],
    });
    expect(screen.getByLabelText("Set alias")).toBeInTheDocument();
  });

  it("opens inline editor when set alias pencil clicked", async () => {
    const { user } = setup({
      identities: [makeIdentity({ alias: null })],
    });
    await user.click(screen.getByLabelText("Set alias"));
    const input = await screen.findByRole("textbox", {
      name: "Identity alias",
    });
    expect(input).toBeInTheDocument();
  });
});

// ─── Context menu — Remove ────────────────────────────────────────

describe("IdentityListPanel — remove", () => {
  it("shows confirmation dialog on Remove click", async () => {
    const { user } = setup({
      identities: [makeIdentity({ alias: "Deletable" })],
    });
    await clickMenuItem(user, "Identity actions", "Remove");
    expect(screen.getByText("Confirm Removal")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Are you sure you want to no longer track this User identity?",
      ),
    ).toBeInTheDocument();
  });

  it("shows correct type in removal dialog for masternode", async () => {
    const { user } = setup({
      identities: [makeIdentity({ identityType: "masternode" })],
    });
    await clickMenuItem(user, "Identity actions", "Remove");
    expect(
      screen.getByText(
        "Are you sure you want to no longer track this Masternode identity?",
      ),
    ).toBeInTheDocument();
  });

  it("calls onRemoveIdentity when confirmed", async () => {
    const onRemoveIdentity = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      identities: [makeIdentity({ id: "del1" })],
      onRemoveIdentity,
    });
    await clickMenuItem(user, "Identity actions", "Remove");
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect(onRemoveIdentity).toHaveBeenCalledWith("del1");
  });

  it("does not call onRemoveIdentity when canceled", async () => {
    const onRemoveIdentity = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onRemoveIdentity,
    });
    await clickMenuItem(user, "Identity actions", "Remove");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onRemoveIdentity).not.toHaveBeenCalled();
  });
});

// ─── Context menu — Actions ───────────────────────────────────────

describe("IdentityListPanel — context menu actions", () => {
  it("shows View Keys in context menu when identity has keys", async () => {
    const onViewKeys = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onViewKeys,
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.getByText("View Keys")).toBeInTheDocument();
  });

  it("does not show View Keys when identity has no keys", async () => {
    const onViewKeys = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ keys: [] })],
      onViewKeys,
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.queryByText("View Keys")).not.toBeInTheDocument();
  });

  it("calls onViewKeys when View Keys clicked", async () => {
    const onViewKeys = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ id: "vk1" })],
      onViewKeys,
    });
    await clickMenuItem(user, "Identity actions", "View Keys");
    expect(onViewKeys).toHaveBeenCalledWith("vk1");
  });

  it("shows Top Up in context menu", async () => {
    const onTopUp = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onTopUp,
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.getByText("Top Up")).toBeInTheDocument();
  });

  it("calls onTopUp when Top Up clicked", async () => {
    const onTopUp = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ id: "tu1" })],
      onTopUp,
    });
    await clickMenuItem(user, "Identity actions", "Top Up");
    expect(onTopUp).toHaveBeenCalledWith("tu1");
  });

  it("shows Withdraw in context menu", async () => {
    const onWithdraw = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onWithdraw,
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.getByText("Withdraw")).toBeInTheDocument();
  });

  it("calls onWithdraw when Withdraw clicked", async () => {
    const onWithdraw = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ id: "wd1", balance: 600_000_000 })],
      onWithdraw,
    });
    await clickMenuItem(user, "Identity actions", "Withdraw");
    expect(onWithdraw).toHaveBeenCalledWith("wd1");
  });

  it("shows Transfer in context menu", async () => {
    const onTransfer = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onTransfer,
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.getByText("Transfer")).toBeInTheDocument();
  });

  it("calls onTransfer when Transfer clicked", async () => {
    const onTransfer = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ id: "tr1", balance: 100_000_000 })],
      onTransfer,
    });
    await clickMenuItem(user, "Identity actions", "Transfer");
    expect(onTransfer).toHaveBeenCalledWith("tr1");
  });

  it("shows Register DPNS Name in context menu", async () => {
    const onRegisterDpns = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onRegisterDpns,
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.getByText("Register DPNS Name")).toBeInTheDocument();
  });

  it("calls onRegisterDpns when Register DPNS Name clicked", async () => {
    const onRegisterDpns = vi.fn();
    const { user } = setup({
      identities: [makeIdentity({ id: "dpns1" })],
      onRegisterDpns,
    });
    await clickMenuItem(user, "Identity actions", "Register DPNS Name");
    expect(onRegisterDpns).toHaveBeenCalledWith("dpns1");
  });

  it("shows Refresh in context menu", async () => {
    const { user } = setup({
      identities: [makeIdentity()],
    });
    await user.click(screen.getByLabelText("Identity actions"));
    expect(screen.getByText("Refresh")).toBeInTheDocument();
  });

  it("calls onRefreshIdentity when Refresh clicked", async () => {
    const onRefreshIdentity = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      identities: [makeIdentity({ id: "ref1" })],
      onRefreshIdentity,
    });
    await clickMenuItem(user, "Identity actions", "Refresh");
    expect(onRefreshIdentity).toHaveBeenCalledWith("ref1");
  });
});

// ─── Reorder ──────────────────────────────────────────────────────

describe("IdentityListPanel — reorder", () => {
  it("renders drag handles for each identity", () => {
    setup({
      identities: [
        makeIdentity({ id: "a", alias: "First" }),
        makeIdentity({ id: "b", alias: "Second" }),
      ],
    });
    const handles = screen.getAllByLabelText("Drag to reorder");
    expect(handles).toHaveLength(2);
  });

  it("passes onReorder callback for drag-and-drop", () => {
    const onReorder = vi.fn().mockResolvedValue(undefined);
    setup({
      identities: [
        makeIdentity({ id: "a", alias: "First" }),
        makeIdentity({ id: "b", alias: "Second" }),
      ],
      onReorder,
    });
    // Drag handles are present and accessible
    const handles = screen.getAllByLabelText("Drag to reorder");
    expect(handles).toHaveLength(2);
  });

  it("renders drag handles with grab cursor styling", () => {
    setup({
      identities: [makeIdentity({ id: "a", alias: "First" })],
    });
    const handle = screen.getByLabelText("Drag to reorder");
    expect(handle).toHaveClass("cursor-grab");
  });

  it("renders one drag handle per identity", () => {
    setup({
      identities: [
        makeIdentity({ id: "a", alias: "First" }),
        makeIdentity({ id: "b", alias: "Second" }),
        makeIdentity({ id: "c", alias: "Third" }),
      ],
    });
    const handles = screen.getAllByLabelText("Drag to reorder");
    expect(handles).toHaveLength(3);
  });

  it("does not render drag handles for empty list", () => {
    setup({ identities: [] });
    expect(screen.queryAllByLabelText("Drag to reorder")).toHaveLength(0);
  });
});

// ─── Refresh all ──────────────────────────────────────────────────

describe("IdentityListPanel — refresh all", () => {
  it("shows refresh all button when identities exist", () => {
    setup({ identities: [makeIdentity()] });
    expect(
      screen.getByLabelText("Refresh all identities"),
    ).toBeInTheDocument();
  });

  it("calls onRefreshAll when refresh all clicked", async () => {
    const onRefreshAll = vi.fn().mockResolvedValue(undefined);
    const { user } = setup({
      identities: [makeIdentity()],
      onRefreshAll,
    });
    await user.click(screen.getByLabelText("Refresh all identities"));
    expect(onRefreshAll).toHaveBeenCalledOnce();
  });

  it("disables refresh all button when refreshing", () => {
    setup({
      identities: [makeIdentity()],
      refreshingAll: true,
    });
    expect(screen.getByLabelText("Refresh all identities")).toBeDisabled();
  });
});

// ─── Balance formatting ────────────────────────────────────────────

describe("IdentityListPanel — balance display", () => {
  it("formats zero balance correctly", () => {
    setup({
      identities: [makeIdentity({ balance: 0 })],
    });
    expect(screen.getByText("0.00000000 DASH")).toBeInTheDocument();
  });

  it("formats large balance correctly", () => {
    setup({
      identities: [makeIdentity({ balance: 1_234_567_890 })],
    });
    expect(screen.getByText("12.34567890 DASH")).toBeInTheDocument();
  });

  it("formats small balance correctly", () => {
    setup({
      identities: [makeIdentity({ balance: 1 })],
    });
    expect(screen.getByText("0.00000001 DASH")).toBeInTheDocument();
  });
});

// ─── Accessibility ─────────────────────────────────────────────────

// ─── Sort controls ────────────────────────────────────────────────

describe("IdentityListPanel — sort controls", () => {
  const alice = makeIdentity({ id: "aaa", alias: "Alice" });
  const bob = makeIdentity({ id: "bbb", alias: "Bob" });
  const identities = [alice, bob];

  it("does not render sort button when onSortChange not provided", () => {
    setup({ identities });
    expect(screen.queryByLabelText("Sort identities")).not.toBeInTheDocument();
  });

  it("renders sort button when onSortChange is provided", () => {
    setup({ identities, onSortChange: vi.fn() });
    expect(screen.getByLabelText("Sort identities")).toBeInTheDocument();
  });

  it("opens sort dropdown with all column options", async () => {
    const onSortChange = vi.fn();
    const { user } = setup({ identities, onSortChange });
    await user.click(screen.getByLabelText("Sort identities"));
    expect(screen.getByText("Alias")).toBeInTheDocument();
    expect(screen.getByText("Balance")).toBeInTheDocument();
    expect(screen.getByText("Type")).toBeInTheDocument();
    expect(screen.getByText("Identity ID")).toBeInTheDocument();
    expect(screen.getByText("Wallet")).toBeInTheDocument();
    expect(screen.getByText("Custom Order")).toBeInTheDocument();
  });

  it("calls onSortChange when a sort column is clicked", async () => {
    const onSortChange = vi.fn();
    const { user } = setup({ identities, onSortChange });
    await user.click(screen.getByLabelText("Sort identities"));
    await user.click(screen.getByText("Balance"));
    expect(onSortChange).toHaveBeenCalledWith("balance");
  });

  it("shows ascending arrow for active sort column", async () => {
    const onSortChange = vi.fn();
    const { user } = setup({
      identities,
      onSortChange,
      sortColumn: "alias",
      sortOrder: "ascending",
      useCustomOrder: false,
    });
    await user.click(screen.getByLabelText("Sort identities"));
    // The Alias menu item should show an ascending arrow
    const aliasItem = screen.getByText("Alias").closest("[role='menuitem']");
    expect(aliasItem).toHaveTextContent("↑");
  });

  it("shows descending arrow for active sort column", async () => {
    const onSortChange = vi.fn();
    const { user } = setup({
      identities,
      onSortChange,
      sortColumn: "balance",
      sortOrder: "descending",
      useCustomOrder: false,
    });
    await user.click(screen.getByLabelText("Sort identities"));
    const balanceItem = screen
      .getByText("Balance")
      .closest("[role='menuitem']");
    expect(balanceItem).toHaveTextContent("↓");
  });

  it("does not show sort arrow when useCustomOrder is true", async () => {
    const onSortChange = vi.fn();
    const { user } = setup({
      identities,
      onSortChange,
      sortColumn: "alias",
      sortOrder: "ascending",
      useCustomOrder: true,
    });
    await user.click(screen.getByLabelText("Sort identities"));
    const aliasItem = screen.getByText("Alias").closest("[role='menuitem']");
    expect(aliasItem).not.toHaveTextContent("↑");
    expect(aliasItem).not.toHaveTextContent("↓");
  });
});

describe("IdentityListPanel — accessibility", () => {
  it("has identity list region with label", () => {
    setup({ identities: [makeIdentity()] });
    expect(
      screen.getByRole("region", { name: "Identity list" }),
    ).toBeInTheDocument();
  });

  it("has listbox role for identity list", () => {
    setup({ identities: [makeIdentity()] });
    expect(
      screen.getByRole("listbox", { name: "Identities" }),
    ).toBeInTheDocument();
  });

  it("identity action buttons have aria labels", () => {
    setup({ identities: [makeIdentity()] });
    expect(screen.getByLabelText("Identity actions")).toBeInTheDocument();
    expect(screen.getByLabelText("Drag to reorder")).toBeInTheDocument();
  });

  it("accepts custom className", () => {
    const { container } = render(
      <TooltipProvider>
        <IdentityListPanel {...defaultProps} className="custom-class" />
      </TooltipProvider>,
    );
    expect(container.firstChild).toHaveClass("custom-class");
  });
});

// ─── Header Create/Load buttons ─────────────────────────────────────

describe("IdentityListPanel — header create/load buttons", () => {
  it("shows create identity button in header when identities exist", () => {
    const onCreateIdentity = vi.fn();
    setup({
      identities: [makeIdentity()],
      onCreateIdentity,
    });
    expect(screen.getByTestId("create-identity-btn")).toBeInTheDocument();
  });

  it("shows load identity button in header when identities exist", () => {
    const onLoadIdentity = vi.fn();
    setup({
      identities: [makeIdentity()],
      onLoadIdentity,
    });
    expect(screen.getByTestId("load-identity-btn")).toBeInTheDocument();
  });

  it("calls onCreateIdentity when header create button clicked", async () => {
    const onCreateIdentity = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onCreateIdentity,
    });
    await user.click(screen.getByTestId("create-identity-btn"));
    expect(onCreateIdentity).toHaveBeenCalledOnce();
  });

  it("calls onLoadIdentity when header load button clicked", async () => {
    const onLoadIdentity = vi.fn();
    const { user } = setup({
      identities: [makeIdentity()],
      onLoadIdentity,
    });
    await user.click(screen.getByTestId("load-identity-btn"));
    expect(onLoadIdentity).toHaveBeenCalledOnce();
  });

  it("does not show create button when onCreateIdentity not provided", () => {
    setup({
      identities: [makeIdentity()],
    });
    expect(screen.queryByTestId("create-identity-btn")).not.toBeInTheDocument();
  });

  it("does not show load button when onLoadIdentity not provided", () => {
    setup({
      identities: [makeIdentity()],
    });
    expect(screen.queryByTestId("load-identity-btn")).not.toBeInTheDocument();
  });
});

// ─── Encoding tooltip ──────────────────────────────────────────────

describe("IdentityListPanel — encoding tooltip", () => {
  it("shows 'UserId (Base58)' in tooltip for user identity", async () => {
    const userIdentity = makeIdentity({
      id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
      identityType: "user",
    });
    const { user } = setup({ identities: [userIdentity] });
    const idSpan = screen.getByText(/GWRSAVFMj/);
    await user.hover(idSpan);
    await waitFor(() => {
      // Radix renders tooltip content twice (visible + sr-only)
      const matches = screen.getAllByText(/UserId \(Base58\)/);
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows 'ProTxHash (Hex)' in tooltip for masternode identity", async () => {
    const mnIdentity = makeIdentity({
      id: "aabbccdd11223344556677889900aabb",
      identityType: "masternode",
    });
    const { user } = setup({ identities: [mnIdentity] });
    const idSpan = screen.getByText(/aabbcc/);
    await user.hover(idSpan);
    await waitFor(() => {
      const matches = screen.getAllByText(/ProTxHash \(Hex\)/);
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows 'ProTxHash (Hex)' in tooltip for evonode identity", async () => {
    const evoIdentity = makeIdentity({
      id: "eeff00112233445566778899aabbccdd",
      identityType: "evonode",
    });
    const { user } = setup({ identities: [evoIdentity] });
    const idSpan = screen.getByText(/eeff00/);
    await user.hover(idSpan);
    await waitFor(() => {
      const matches = screen.getAllByText(/ProTxHash \(Hex\)/);
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });
});
