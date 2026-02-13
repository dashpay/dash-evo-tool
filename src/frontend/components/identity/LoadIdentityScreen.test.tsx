import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  LoadIdentityScreen,
  type LoadIdentityStatus,
} from "./LoadIdentityScreen";
import type { WalletDto, NetworkDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeWallet(overrides: Partial<WalletDto> = {}): WalletDto {
  return {
    seedHash: "wallet1seedhash",
    usesPassword: false,
    alias: "Test Wallet",
    isMain: true,
    confirmedBalance: 10_000_000_00,
    unconfirmedBalance: 0,
    totalBalance: 10_000_000_00,
    addresses: [],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [0],
    passwordHint: null,
    ...overrides,
  };
}

const defaultProps = {
  wallets: [makeWallet()],
  status: { type: "form" } as LoadIdentityStatus,
  onLoadById: vi.fn(),
  onSearchFromWallet: vi.fn(),
  onSearchUpToIndex: vi.fn(),
  onSearchByDpnsName: vi.fn(),
  onDismissError: vi.fn(),
  onBack: vi.fn(),
  onLoadAnother: vi.fn(),
};

function setup(
  props: Partial<Parameters<typeof LoadIdentityScreen>[0]> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <LoadIdentityScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("LoadIdentityScreen — header", () => {
  it("renders title", () => {
    setup();
    expect(
      screen.getByText("Load Existing Identity"),
    ).toBeInTheDocument();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /back/i }));
    expect(props.onBack).toHaveBeenCalled();
  });
});

// ─── Tabs ───────────────────────────────────────────────────────────

describe("LoadIdentityScreen — tabs", () => {
  it("renders three tabs", () => {
    setup();
    expect(screen.getByText("By Identity ID")).toBeInTheDocument();
    expect(screen.getByText("By Wallet")).toBeInTheDocument();
    expect(screen.getByText("By DPNS Name")).toBeInTheDocument();
  });

  it("defaults to By Identity ID tab", () => {
    setup();
    expect(
      screen.getByPlaceholderText(/Enter identity ID/),
    ).toBeInTheDocument();
  });

  it("can switch to By Wallet tab", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By Wallet"));
    expect(screen.getByLabelText(/Wallet/i)).toBeInTheDocument();
  });

  it("can switch to By DPNS Name tab", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    expect(screen.getByLabelText(/DPNS Username/i)).toBeInTheDocument();
  });
});

// ─── By Identity ID ─────────────────────────────────────────────────

describe("LoadIdentityScreen — By Identity ID", () => {
  it("has identity ID input", () => {
    setup();
    expect(
      screen.getByPlaceholderText(/Enter identity ID/),
    ).toBeInTheDocument();
  });

  it("has alias input", () => {
    setup();
    expect(
      screen.getByPlaceholderText(/Local display name/),
    ).toBeInTheDocument();
  });

  it("submit disabled when ID empty", () => {
    setup();
    const btn = screen.getByRole("button", { name: /Load Identity/i });
    expect(btn).toBeDisabled();
  });

  it("submit disabled with invalid short ID", async () => {
    const { user } = setup();
    await user.type(
      screen.getByPlaceholderText(/Enter identity ID/),
      "abc",
    );
    const btn = screen.getByRole("button", { name: /Load Identity/i });
    expect(btn).toBeDisabled();
  });

  it("shows validation error for invalid ID", async () => {
    const { user } = setup();
    await user.type(
      screen.getByPlaceholderText(/Enter identity ID/),
      "xyz!!!",
    );
    expect(screen.getByText(/valid 64-character hex/i)).toBeInTheDocument();
  });

  it("submit enabled with valid 64-char hex ID", async () => {
    const { user } = setup();
    const hexId =
      "aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd";
    await user.type(
      screen.getByPlaceholderText(/Enter identity ID/),
      hexId,
    );
    const btn = screen.getByRole("button", { name: /Load Identity/i });
    expect(btn).not.toBeDisabled();
  });

  it("calls onLoadById with correct params", async () => {
    const { user, props } = setup();
    const hexId =
      "aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd";
    await user.type(
      screen.getByPlaceholderText(/Enter identity ID/),
      hexId,
    );
    await user.type(
      screen.getByPlaceholderText(/Local display name/),
      "My Identity",
    );
    await user.click(
      screen.getByRole("button", { name: /Load Identity/i }),
    );
    expect(props.onLoadById).toHaveBeenCalledWith(
      expect.objectContaining({
        identityId: hexId,
        identityType: "user",
        alias: "My Identity",
        deriveKeysFromWallets: false,
      }),
    );
  });

  it("has advanced options toggle", () => {
    setup();
    expect(
      screen.getByText(/Show Advanced Options/i),
    ).toBeInTheDocument();
  });

  it("shows identity type selector in advanced mode", async () => {
    const { user } = setup();
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.getByText("Identity Type")).toBeInTheDocument();
  });

  it("shows derive from wallet checkbox in advanced mode", async () => {
    const { user } = setup();
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(
      screen.getByText(/Try to derive private keys/i),
    ).toBeInTheDocument();
  });
});

// ─── By Wallet ──────────────────────────────────────────────────────

describe("LoadIdentityScreen — By Wallet", () => {
  it("shows single wallet info", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By Wallet"));
    expect(screen.getByText(/Test Wallet/)).toBeInTheDocument();
  });

  it("shows no wallets message when empty", async () => {
    const { user } = setup({ wallets: [] });
    await user.click(screen.getByText("By Wallet"));
    expect(
      screen.getByText(/No wallets available/),
    ).toBeInTheDocument();
  });

  it("has identity index input", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By Wallet"));
    expect(screen.getByLabelText(/Identity index/i)).toBeInTheDocument();
  });

  it("submit button says Search For Identity", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By Wallet"));
    expect(
      screen.getByRole("button", { name: /Search For Identity/i }),
    ).toBeInTheDocument();
  });

  it("calls onSearchFromWallet with specific index", async () => {
    const { user, props } = setup();
    await user.click(screen.getByText("By Wallet"));
    // Default index is 0
    await user.click(
      screen.getByRole("button", { name: /Search For Identity/i }),
    );
    expect(props.onSearchFromWallet).toHaveBeenCalledWith({
      walletSeedHash: "wallet1seedhash",
      identityIndex: 0,
    });
  });

  it("can change identity index", async () => {
    const { user, props } = setup();
    await user.click(screen.getByText("By Wallet"));
    const indexInput = screen.getByLabelText(
      /Identity index/i,
    ) as HTMLInputElement;
    await user.clear(indexInput);
    await user.type(indexInput, "5");
    await user.click(
      screen.getByRole("button", { name: /Search For Identity/i }),
    );
    expect(props.onSearchFromWallet).toHaveBeenCalledWith({
      walletSeedHash: "wallet1seedhash",
      identityIndex: 5,
    });
  });
});

// ─── By DPNS Name ───────────────────────────────────────────────────

describe("LoadIdentityScreen — By DPNS Name", () => {
  it("has DPNS name input", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    expect(screen.getByPlaceholderText("alice")).toBeInTheDocument();
  });

  it("shows .dash suffix", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    expect(screen.getByText(".dash")).toBeInTheDocument();
  });

  it("submit disabled with short name", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    await user.type(screen.getByPlaceholderText("alice"), "ab");
    const btn = screen.getByRole("button", {
      name: /Search by Username/i,
    });
    expect(btn).toBeDisabled();
  });

  it("shows validation hint for short name", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    await user.type(screen.getByPlaceholderText("alice"), "ab");
    expect(
      screen.getByText(/at least 3 characters/i),
    ).toBeInTheDocument();
  });

  it("submit enabled with valid name", async () => {
    const { user } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    await user.type(screen.getByPlaceholderText("alice"), "alice");
    const btn = screen.getByRole("button", {
      name: /Search by Username/i,
    });
    expect(btn).not.toBeDisabled();
  });

  it("calls onSearchByDpnsName with correct params", async () => {
    const { user, props } = setup();
    await user.click(screen.getByText("By DPNS Name"));
    await user.type(screen.getByPlaceholderText("alice"), "bob");
    await user.click(
      screen.getByRole("button", { name: /Search by Username/i }),
    );
    expect(props.onSearchByDpnsName).toHaveBeenCalledWith({
      name: "bob",
      walletSeedHash: null,
    });
  });
});

// ─── Status screens ────────────────────────────────────────────────

describe("LoadIdentityScreen — status screens", () => {
  it("shows success screen", () => {
    setup({
      status: { type: "success", message: "Identity loaded successfully" },
    });
    expect(screen.getByText("Identity Loaded!")).toBeInTheDocument();
    expect(
      screen.getByText("Identity loaded successfully"),
    ).toBeInTheDocument();
  });

  it("has Load Another and Back buttons on success", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByRole("button", { name: /Load Another/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Back to Identities/i }),
    ).toBeInTheDocument();
  });

  it("calls onLoadAnother", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /Load Another/i }),
    );
    expect(props.onLoadAnother).toHaveBeenCalled();
  });

  it("calls onBack from success", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /Back to Identities/i }),
    );
    expect(props.onBack).toHaveBeenCalled();
  });

  it("shows error screen", () => {
    setup({ status: { type: "error", message: "Identity not found" } });
    expect(
      screen.getByText("Failed to Load Identity"),
    ).toBeInTheDocument();
    // InlineError translates raw messages via translateError()
    expect(screen.getByText("The requested identity was not found on Platform.")).toBeInTheDocument();
  });

  it("calls onDismissError from error screen", async () => {
    const { user, props } = setup({
      status: { type: "error", message: "fail" },
    });
    await user.click(screen.getByRole("button", { name: /Try Again/i }));
    expect(props.onDismissError).toHaveBeenCalled();
  });

  it("shows loading screen", () => {
    setup({ status: { type: "loading", startedAt: Date.now() - 3000 } });
    expect(screen.getByText("Searching…")).toBeInTheDocument();
    expect(screen.getByText(/Time elapsed/)).toBeInTheDocument();
  });

  it("shows progress message during loading when provided", () => {
    setup({
      status: {
        type: "loading",
        startedAt: Date.now() - 5000,
        progressMessage: "Searching index 3 of 10...",
      },
    });
    expect(screen.getByText("Searching…")).toBeInTheDocument();
    expect(
      screen.getByText(/Searching index 3 of 10\.\.\./),
    ).toBeInTheDocument();
    // Should NOT show "Time elapsed:" prefix when progress message is present
    expect(screen.queryByText(/Time elapsed/)).not.toBeInTheDocument();
  });

  it("shows elapsed time alongside progress message", () => {
    setup({
      status: {
        type: "loading",
        startedAt: Date.now() - 65000,
        progressMessage: "Searching index 5 of 10...",
      },
    });
    // Progress message should include the elapsed time in parentheses
    const progressEl = screen.getByText(/Searching index 5 of 10/);
    expect(progressEl.textContent).toMatch(/\(1m \d+s\)/);
  });

  it("shows generic loading when no progress message", () => {
    setup({
      status: {
        type: "loading",
        startedAt: Date.now() - 3000,
      },
    });
    expect(screen.getByText(/Time elapsed/)).toBeInTheDocument();
    expect(
      screen.queryByText(/Searching index/),
    ).not.toBeInTheDocument();
  });
});

// ─── Advanced options — By Identity ID ──────────────────────────────

describe("LoadIdentityScreen — advanced options", () => {
  it("toggles advanced options visibility", async () => {
    const { user } = setup();
    expect(
      screen.queryByText("Identity Type"),
    ).not.toBeInTheDocument();
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.getByText("Identity Type")).toBeInTheDocument();
    await user.click(screen.getByText(/Hide Advanced Options/i));
    expect(
      screen.queryByText("Identity Type"),
    ).not.toBeInTheDocument();
  });

  it("shows manual key inputs for user type in advanced", async () => {
    const { user } = setup();
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(
      screen.getByText(/Private Keys \(Hex or WIF\)/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Add key manually/i }),
    ).toBeInTheDocument();
  });

  it("can add and remove manual keys", async () => {
    const { user } = setup();
    await user.click(screen.getByText(/Show Advanced Options/i));
    await user.click(
      screen.getByRole("button", { name: /Add key manually/i }),
    );
    const keyInputs = screen.getAllByPlaceholderText(/Private key/);
    expect(keyInputs).toHaveLength(1);

    // Remove it
    await user.click(
      screen.getByRole("button", { name: /Remove key 1/i }),
    );
    expect(
      screen.queryByPlaceholderText(/Private key/),
    ).not.toBeInTheDocument();
  });

  it("shows derive from wallet checkbox", async () => {
    const { user } = setup();
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(
      screen.getByText(/Try to derive private keys/i),
    ).toBeInTheDocument();
  });
});

// ─── Multiple wallets ──────────────────────────────────────────────

describe("LoadIdentityScreen — multiple wallets", () => {
  it("shows wallet selector for wallet tab with multiple wallets", async () => {
    const wallet2 = makeWallet({
      seedHash: "wallet2seedhash",
      alias: "Second Wallet",
    });
    const { user } = setup({ wallets: [makeWallet(), wallet2] });
    await user.click(screen.getByText("By Wallet"));
    // With 2 wallets it should show a combobox trigger, not plain text
    // The first wallet should be pre-selected in the trigger
    expect(screen.getByText(/Test Wallet/)).toBeInTheDocument();
  });
});

// ─── Testnet helper buttons ─────────────────────────────────────────

describe("LoadIdentityScreen — testnet helpers", () => {
  it("does not show fill buttons without network prop", async () => {
    const { user } = setup();
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.queryByTestId("fill-random-hpmn")).not.toBeInTheDocument();
    expect(screen.queryByTestId("fill-random-masternode")).not.toBeInTheDocument();
  });

  it("does not show fill buttons on mainnet", async () => {
    const { user } = setup({ network: "dash" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.queryByTestId("fill-random-hpmn")).not.toBeInTheDocument();
    expect(screen.queryByTestId("fill-random-masternode")).not.toBeInTheDocument();
  });

  it("shows fill buttons on testnet in advanced mode", async () => {
    const { user } = setup({ network: "testnet" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.getByTestId("fill-random-hpmn")).toBeInTheDocument();
    expect(screen.getByTestId("fill-random-masternode")).toBeInTheDocument();
  });

  it("shows fill buttons on devnet in advanced mode", async () => {
    const { user } = setup({ network: "devnet" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.getByTestId("fill-random-hpmn")).toBeInTheDocument();
    expect(screen.getByTestId("fill-random-masternode")).toBeInTheDocument();
  });

  it("shows fill buttons on regtest in advanced mode", async () => {
    const { user } = setup({ network: "regtest" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    expect(screen.getByTestId("fill-random-hpmn")).toBeInTheDocument();
    expect(screen.getByTestId("fill-random-masternode")).toBeInTheDocument();
  });

  it("does not show fill buttons when advanced is collapsed", () => {
    setup({ network: "testnet" as NetworkDto });
    expect(screen.queryByTestId("fill-random-hpmn")).not.toBeInTheDocument();
  });

  it("Fill Random HPMN sets identity type to evonode and fills ID", async () => {
    const { user, props } = setup({ network: "testnet" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    await user.click(screen.getByTestId("fill-random-hpmn"));

    // The identity ID field should now have a 64-char hex value
    const idInput = screen.getByPlaceholderText(/Enter identity ID/) as HTMLInputElement;
    expect(idInput.value).toMatch(/^[0-9a-f]{64}$/);

    // Submit should work (field is valid) and show evonode type
    await user.click(screen.getByRole("button", { name: /Load Identity/i }));
    expect(props.onLoadById).toHaveBeenCalledWith(
      expect.objectContaining({
        identityType: "evonode",
      }),
    );
  });

  it("Fill Random Masternode sets identity type to masternode and fills ID", async () => {
    const { user, props } = setup({ network: "testnet" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    await user.click(screen.getByTestId("fill-random-masternode"));

    // The identity ID field should now have a 64-char hex value
    const idInput = screen.getByPlaceholderText(/Enter identity ID/) as HTMLInputElement;
    expect(idInput.value).toMatch(/^[0-9a-f]{64}$/);

    // Submit should work and show masternode type
    await user.click(screen.getByRole("button", { name: /Load Identity/i }));
    expect(props.onLoadById).toHaveBeenCalledWith(
      expect.objectContaining({
        identityType: "masternode",
      }),
    );
  });

  it("Fill Random HPMN generates an alias with HPMN prefix", async () => {
    const { user, props } = setup({ network: "testnet" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    await user.click(screen.getByTestId("fill-random-hpmn"));
    await user.click(screen.getByRole("button", { name: /Load Identity/i }));
    expect(props.onLoadById).toHaveBeenCalledWith(
      expect.objectContaining({
        alias: expect.stringMatching(/^HPMN-\d+$/),
      }),
    );
  });

  it("Fill Random Masternode generates an alias with MN prefix", async () => {
    const { user, props } = setup({ network: "testnet" as NetworkDto });
    await user.click(screen.getByText(/Show Advanced Options/i));
    await user.click(screen.getByTestId("fill-random-masternode"));
    await user.click(screen.getByRole("button", { name: /Load Identity/i }));
    expect(props.onLoadById).toHaveBeenCalledWith(
      expect.objectContaining({
        alias: expect.stringMatching(/^MN-\d+$/),
      }),
    );
  });
});
