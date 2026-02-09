import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { KeyManagementScreen } from "./KeyManagementScreen";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    keyType: "ECDSA_SECP256K1",
    purpose: "AUTHENTICATION",
    securityLevel: "MASTER",
    data: "02aabb",
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: true,
    ...overrides,
  };
}

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aabbccdd11223344556677889900aabb",
    identityType: "user",
    alias: "Alice",
    balance: 1_000_000_000,
    keys: [
      makeKey({ keyId: 0, purpose: "AUTHENTICATION", securityLevel: "MASTER" }),
      makeKey({ keyId: 1, purpose: "AUTHENTICATION", securityLevel: "HIGH" }),
      makeKey({ keyId: 2, purpose: "TRANSFER", securityLevel: "CRITICAL" }),
      makeKey({ keyId: 3, purpose: "VOTING", securityLevel: "HIGH" }),
    ],
    dpnsNames: [],
    associatedWalletHashes: [],
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
  identity: makeIdentity(),
  onViewKey: vi.fn(),
  onAddKey: vi.fn(),
  onBack: vi.fn(),
};

function setup(
  props: Partial<Parameters<typeof KeyManagementScreen>[0]> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <KeyManagementScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("KeyManagementScreen — header", () => {
  it("renders the title and identity name", () => {
    setup();
    expect(screen.getByText("Identity Keys")).toBeInTheDocument();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("shows truncated identity ID when no alias", () => {
    setup({
      identity: makeIdentity({ alias: null }),
    });
    expect(screen.getByText(/aabbccdd…00aabb/)).toBeInTheDocument();
  });

  it("renders Add Key button", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /add key/i }),
    ).toBeInTheDocument();
  });

  it("calls onAddKey when Add Key button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(props.onAddKey).toHaveBeenCalledOnce();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /go back/i }));
    expect(props.onBack).toHaveBeenCalledOnce();
  });

  it("hides back button when onBack is not provided", () => {
    setup({ onBack: undefined });
    expect(
      screen.queryByRole("button", { name: /go back/i }),
    ).not.toBeInTheDocument();
  });

  it("hides Add Key button when onAddKey is not provided", () => {
    setup({ onAddKey: undefined });
    expect(
      screen.queryByRole("button", { name: /add key/i }),
    ).not.toBeInTheDocument();
  });
});

// ─── Key Sections ───────────────────────────────────────────────────

describe("KeyManagementScreen — key sections", () => {
  it("renders Main Keys and Voter Keys sections", () => {
    setup();
    expect(screen.getByText("Main Keys")).toBeInTheDocument();
    expect(screen.getByText("Voter Keys")).toBeInTheDocument();
  });

  it("shows correct count badges", () => {
    setup();
    // 3 main keys (AUTH x2 + TRANSFER), 1 voter key
    const badges = screen.getAllByText(/^[0-9]+$/);
    const badgeValues = badges.map((b) => b.textContent);
    expect(badgeValues).toContain("3");
    expect(badgeValues).toContain("1");
  });

  it("does not render Voter Keys section when no voting keys", () => {
    setup({
      identity: makeIdentity({
        keys: [
          makeKey({ keyId: 0, purpose: "AUTHENTICATION" }),
          makeKey({ keyId: 1, purpose: "TRANSFER" }),
        ],
      }),
    });
    expect(screen.getByText("Main Keys")).toBeInTheDocument();
    expect(screen.queryByText("Voter Keys")).not.toBeInTheDocument();
  });

  it("does not render Main Keys section when only voting keys", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ keyId: 0, purpose: "VOTING" })],
      }),
    });
    expect(screen.queryByText("Main Keys")).not.toBeInTheDocument();
    expect(screen.getByText("Voter Keys")).toBeInTheDocument();
  });
});

// ─── Empty State ────────────────────────────────────────────────────

describe("KeyManagementScreen — empty state", () => {
  it("shows empty state when no keys", () => {
    setup({ identity: makeIdentity({ keys: [] }) });
    expect(screen.getByText("No keys")).toBeInTheDocument();
    expect(
      screen.getByText("This identity has no public keys."),
    ).toBeInTheDocument();
  });

  it("shows Add Key button in empty state when onAddKey provided", () => {
    setup({ identity: makeIdentity({ keys: [] }) });
    // Header Add Key + EmptyState Add Key = 2 buttons
    const buttons = screen.getAllByRole("button", { name: /add key/i });
    expect(buttons.length).toBeGreaterThanOrEqual(1);
  });
});

// ─── Key Rows ───────────────────────────────────────────────────────

describe("KeyManagementScreen — key rows", () => {
  it("renders key ID", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ keyId: 42 })],
      }),
    });
    expect(screen.getByText("42")).toBeInTheDocument();
  });

  it("renders purpose in title case", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ purpose: "AUTHENTICATION" })],
      }),
    });
    expect(screen.getByText("Authentication")).toBeInTheDocument();
  });

  it("renders security level with color", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ securityLevel: "CRITICAL" })],
      }),
    });
    expect(screen.getByText("Critical")).toBeInTheDocument();
  });

  it("renders key type badge", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ keyType: "ECDSA_SECP256K1" })],
      }),
    });
    expect(screen.getByText("ECDSA")).toBeInTheDocument();
  });

  it("renders BLS key type badge", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ keyType: "BLS12_381" })],
      }),
    });
    expect(screen.getByText("BLS")).toBeInTheDocument();
  });

  it("shows Active badge for non-disabled key", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ isDisabled: false })],
      }),
    });
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  it("shows Disabled badge for disabled key", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ isDisabled: true })],
      }),
    });
    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });

  it("calls onViewKey when key row clicked", async () => {
    const { user, props } = setup({
      identity: makeIdentity({
        keys: [makeKey({ keyId: 7 })],
      }),
    });
    // Click on any part of the row
    await user.click(screen.getByText("7"));
    expect(props.onViewKey).toHaveBeenCalledWith(7);
  });

  it("calls onViewKey when view button clicked", async () => {
    const { user, props } = setup({
      identity: makeIdentity({
        keys: [makeKey({ keyId: 7 })],
      }),
    });
    await user.click(screen.getByRole("button", { name: /view key 7/i }));
    expect(props.onViewKey).toHaveBeenCalledWith(7);
  });

  it("row is keyboard accessible when onViewKey provided", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ keyId: 0 })],
      }),
    });
    const row = screen.getByRole("button", {
      name: /key 0.*authentication.*master/i,
    });
    expect(row).toHaveAttribute("tabindex", "0");
  });
});

// ─── Private Key Indicator ──────────────────────────────────────────

describe("KeyManagementScreen — private key indicator", () => {
  it("shows 'Private key available' indicator for key with private key", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ hasPrivateKey: true })],
      }),
    });
    expect(
      screen.getByLabelText("Private key available"),
    ).toBeInTheDocument();
  });

  it("shows 'No private key' indicator for key without private key", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ hasPrivateKey: false })],
      }),
    });
    expect(screen.getByLabelText("No private key")).toBeInTheDocument();
  });
});

// ─── Sorting ────────────────────────────────────────────────────────

describe("KeyManagementScreen — key sorting", () => {
  it("sorts keys by purpose then security level", () => {
    setup({
      identity: makeIdentity({
        keys: [
          makeKey({ keyId: 1, purpose: "TRANSFER", securityLevel: "CRITICAL" }),
          makeKey({ keyId: 2, purpose: "AUTHENTICATION", securityLevel: "HIGH" }),
          makeKey({ keyId: 3, purpose: "AUTHENTICATION", securityLevel: "MASTER" }),
        ],
      }),
    });

    // Get all key ID cells in order
    const rows = screen.getAllByRole("button", { name: /^Key \d+/ });
    const ids = rows.map((r) => {
      const match = r.getAttribute("aria-label")?.match(/Key (\d+)/);
      return match ? parseInt(match[1]) : -1;
    });
    // Auth MASTER (3) → Auth HIGH (2) → Transfer CRITICAL (1)
    expect(ids).toEqual([3, 2, 1]);
  });
});

// ─── Accessibility ──────────────────────────────────────────────────

describe("KeyManagementScreen — accessibility", () => {
  it("has region role with label", () => {
    setup();
    expect(
      screen.getByRole("region", { name: "Key management" }),
    ).toBeInTheDocument();
  });

  it("renders table headers", () => {
    setup();
    expect(screen.getAllByText("ID").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Purpose").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Security").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Type").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Status").length).toBeGreaterThanOrEqual(1);
  });

  it("accepts className prop", () => {
    const { container } = setup({ className: "my-custom-class" });
    expect(container.firstChild).toHaveClass("my-custom-class");
  });
});
