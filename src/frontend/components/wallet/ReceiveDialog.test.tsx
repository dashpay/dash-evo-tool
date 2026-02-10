/**
 * ReceiveDialog component tests — migrated to centralized test infrastructure.
 *
 * Changes from original:
 * - Removed Radix pointer polyfills (now in test/setup.ts)
 * - No other changes needed — this component doesn't use IPC or router
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ReceiveDialog, type ReceiveAddress } from "./ReceiveDialog";

// ── Helpers ──────────────────────────────────────────────────────────
// Pointer capture polyfills are now in test/setup.ts (centralized)

const coreAddresses: ReceiveAddress[] = [
  { address: "yXbR1pQ7vE8kSfPnA3mWoJ4gH2cT6dL9u", balance: 150000000 },
  { address: "yK9tU3wN7dF5bH2xJ0eR4mQ6aC8sG1iP", balance: 0 },
];

const platformAddresses: ReceiveAddress[] = [
  { address: "tevo1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", balance: 5000000000 },
];

function renderDialog(props: Partial<React.ComponentProps<typeof ReceiveDialog>> = {}) {
  const defaultProps: React.ComponentProps<typeof ReceiveDialog> = {
    open: true,
    onOpenChange: vi.fn(),
    walletName: "My Wallet",
    walletType: "hd",
    coreAddresses,
    platformAddresses,
    onNewCoreAddress: vi.fn(),
    onNewPlatformAddress: vi.fn(),
    ...props,
  };
  return render(<ReceiveDialog {...defaultProps} />);
}

// ── Tests ────────────────────────────────────────────────────────────

describe("ReceiveDialog", () => {
  // ── Rendering ────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders dialog title with wallet name", () => {
      renderDialog();
      expect(screen.getByText("Receive — My Wallet")).toBeInTheDocument();
    });

    it("does not render when closed", () => {
      renderDialog({ open: false });
      expect(screen.queryByText("Receive — My Wallet")).not.toBeInTheDocument();
    });

    it("calls onOpenChange when dialog closes", async () => {
      const user = userEvent.setup();
      const onOpenChange = vi.fn();
      renderDialog({ onOpenChange });
      // Press Escape to close
      await user.keyboard("{Escape}");
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });
  });

  // ── HD Wallet (Core + Platform Tabs) ─────────────────────────────

  describe("HD wallet tabs", () => {
    it("shows Core and Platform tabs for HD wallets", () => {
      renderDialog();
      expect(screen.getByRole("tab", { name: "Core" })).toBeInTheDocument();
      expect(screen.getByRole("tab", { name: "Platform" })).toBeInTheDocument();
    });

    it("defaults to Core tab", () => {
      renderDialog();
      const coreTab = screen.getByRole("tab", { name: "Core" });
      expect(coreTab).toHaveAttribute("data-state", "active");
    });

    it("can switch to Platform tab", async () => {
      const user = userEvent.setup();
      renderDialog();
      await user.click(screen.getByRole("tab", { name: "Platform" }));
      const platformTab = screen.getByRole("tab", { name: "Platform" });
      expect(platformTab).toHaveAttribute("data-state", "active");
    });

    it("hides Platform tab when no platform addresses", () => {
      renderDialog({ platformAddresses: [] });
      expect(screen.queryByRole("tab", { name: "Platform" })).not.toBeInTheDocument();
      expect(screen.queryByRole("tab", { name: "Core" })).not.toBeInTheDocument();
    });
  });

  // ── Single-Key Wallet ────────────────────────────────────────────

  describe("single-key wallet", () => {
    it("shows only Core content without tabs", () => {
      renderDialog({
        walletType: "singleKey",
        coreAddresses: [coreAddresses[0]],
        platformAddresses: [],
      });
      // No tabs should be visible
      expect(screen.queryByRole("tab")).not.toBeInTheDocument();
      // Core address should be shown
      expect(
        screen.getByText(coreAddresses[0].address),
      ).toBeInTheDocument();
    });
  });

  // ── QR Code ──────────────────────────────────────────────────────

  describe("QR code", () => {
    it("renders QR code with aria-label", () => {
      renderDialog();
      const qrContainer = screen.getByRole("img", {
        name: /QR code for address/,
      });
      expect(qrContainer).toBeInTheDocument();
    });

    it("QR code aria-label includes current address", () => {
      renderDialog();
      expect(
        screen.getByRole("img", {
          name: `QR code for address ${coreAddresses[0].address}`,
        }),
      ).toBeInTheDocument();
    });

    it("renders SVG QR code element", () => {
      renderDialog();
      const qrContainer = screen.getByRole("img", {
        name: /QR code for address/,
      });
      const svg = qrContainer.querySelector("svg");
      expect(svg).toBeInTheDocument();
    });
  });

  // ── Address Display ──────────────────────────────────────────────

  describe("address display", () => {
    it("shows full address in monospace", () => {
      renderDialog({ coreAddresses: [coreAddresses[0]], platformAddresses: [] });
      const codeEl = screen.getByText(coreAddresses[0].address);
      expect(codeEl).toBeInTheDocument();
      expect(codeEl.tagName.toLowerCase()).toBe("code");
    });

    it("shows copy button for address", () => {
      renderDialog({ coreAddresses: [coreAddresses[0]], platformAddresses: [] });
      expect(
        screen.getByRole("button", { name: /copy/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Balance Display ──────────────────────────────────────────────

  describe("balance display", () => {
    it("shows Core balance in DASH (duffs conversion)", () => {
      renderDialog({ coreAddresses: [coreAddresses[0]], platformAddresses: [] });
      // 150000000 duffs = 1.50000000 DASH
      expect(screen.getByText("1.50000000 DASH")).toBeInTheDocument();
    });

    it("shows zero balance correctly", () => {
      renderDialog({
        coreAddresses: [{ address: "yTest123", balance: 0 }],
        platformAddresses: [],
      });
      expect(screen.getByText("0.00000000 DASH")).toBeInTheDocument();
    });

    it("shows Platform balance in DASH (credits conversion)", async () => {
      const user = userEvent.setup();
      renderDialog({
        platformAddresses: [
          { address: "tevo1abc", balance: 5000000000 },
        ],
      });
      // Switch to Platform tab
      await user.click(screen.getByRole("tab", { name: "Platform" }));
      // 5000000000 credits / 1000 = 5000000 duffs = 0.05000000 DASH
      expect(screen.getByText("0.05000000 DASH")).toBeInTheDocument();
    });

    it("displays Balance label", () => {
      renderDialog({ coreAddresses: [coreAddresses[0]], platformAddresses: [] });
      expect(screen.getByText("Balance")).toBeInTheDocument();
    });
  });

  // ── Address Selector (Multiple Addresses) ────────────────────────

  describe("address selector", () => {
    it("shows dropdown when multiple addresses exist", () => {
      renderDialog({ platformAddresses: [] });
      expect(
        screen.getByRole("combobox", { name: /select address/i }),
      ).toBeInTheDocument();
    });

    it("does not show dropdown for single address", () => {
      renderDialog({
        coreAddresses: [coreAddresses[0]],
        platformAddresses: [],
      });
      expect(
        screen.queryByRole("combobox", { name: /select address/i }),
      ).not.toBeInTheDocument();
    });

    it("switches address when selecting from dropdown", async () => {
      const user = userEvent.setup();
      renderDialog({ platformAddresses: [] });

      // Initially shows first address
      expect(
        screen.getByText(coreAddresses[0].address),
      ).toBeInTheDocument();

      // Open dropdown and select second address
      await user.click(
        screen.getByRole("combobox", { name: /select address/i }),
      );
      // Find the second option (with the truncated address)
      const options = screen.getAllByRole("option");
      await user.click(options[1]);

      // Now shows second address
      expect(
        screen.getByText(coreAddresses[1].address),
      ).toBeInTheDocument();
    });
  });

  // ── New Address Button ───────────────────────────────────────────

  describe("new address button", () => {
    it("renders New Address button when onNewCoreAddress is provided", () => {
      renderDialog({ platformAddresses: [] });
      expect(
        screen.getByRole("button", { name: /new address/i }),
      ).toBeInTheDocument();
    });

    it("does not render New Address button without callback", () => {
      renderDialog({
        onNewCoreAddress: undefined,
        onNewPlatformAddress: undefined,
        platformAddresses: [],
      });
      expect(
        screen.queryByRole("button", { name: /new address/i }),
      ).not.toBeInTheDocument();
    });

    it("calls onNewCoreAddress when clicked", async () => {
      const user = userEvent.setup();
      const onNewCoreAddress = vi.fn();
      renderDialog({ onNewCoreAddress, platformAddresses: [] });
      await user.click(
        screen.getByRole("button", { name: /new address/i }),
      );
      expect(onNewCoreAddress).toHaveBeenCalledOnce();
    });

    it("shows generating state", () => {
      renderDialog({ generatingAddress: true, platformAddresses: [] });
      const btn = screen.getByRole("button", { name: /generating/i });
      expect(btn).toBeDisabled();
    });

    it("calls onNewPlatformAddress on Platform tab", async () => {
      const user = userEvent.setup();
      const onNewPlatformAddress = vi.fn();
      renderDialog({ onNewPlatformAddress });
      await user.click(screen.getByRole("tab", { name: "Platform" }));
      await user.click(
        screen.getByRole("button", { name: /new address/i }),
      );
      expect(onNewPlatformAddress).toHaveBeenCalledOnce();
    });
  });

  // ── Info Text ────────────────────────────────────────────────────

  describe("info text", () => {
    it("shows Core info text", () => {
      renderDialog({ platformAddresses: [] });
      expect(
        screen.getByText("Send Dash to this address to fund your wallet."),
      ).toBeInTheDocument();
    });

    it("shows Platform info text on Platform tab", async () => {
      const user = userEvent.setup();
      renderDialog();
      await user.click(screen.getByRole("tab", { name: "Platform" }));
      expect(
        screen.getByText("Send credits to this Platform address."),
      ).toBeInTheDocument();
    });
  });

  // ── Empty State ──────────────────────────────────────────────────

  describe("empty addresses", () => {
    it("shows empty state when no Core addresses", () => {
      renderDialog({ coreAddresses: [], platformAddresses: [] });
      expect(screen.getByText("No addresses available.")).toBeInTheDocument();
    });

    it("shows Generate Address button in empty state", () => {
      renderDialog({ coreAddresses: [], platformAddresses: [] });
      expect(
        screen.getByRole("button", { name: /generate address/i }),
      ).toBeInTheDocument();
    });

    it("calls onNewCoreAddress from empty state", async () => {
      const user = userEvent.setup();
      const onNewCoreAddress = vi.fn();
      renderDialog({
        coreAddresses: [],
        platformAddresses: [],
        onNewCoreAddress,
      });
      await user.click(
        screen.getByRole("button", { name: /generate address/i }),
      );
      expect(onNewCoreAddress).toHaveBeenCalledOnce();
    });

    it("disables Generate button when generating", () => {
      renderDialog({
        coreAddresses: [],
        platformAddresses: [],
        generatingAddress: true,
      });
      const btn = screen.getByRole("button", { name: /generating/i });
      expect(btn).toBeDisabled();
    });
  });

  // ── Edge Cases ───────────────────────────────────────────────────

  describe("edge cases", () => {
    it("handles very long addresses gracefully", () => {
      const longAddr = "tevo1" + "a".repeat(100);
      renderDialog({
        coreAddresses: [{ address: longAddr, balance: 100 }],
        platformAddresses: [],
      });
      // Address should be rendered with break-all styling
      const codeEl = screen.getByText(longAddr);
      expect(codeEl).toBeInTheDocument();
    });

    it("handles large balance values", () => {
      renderDialog({
        coreAddresses: [
          { address: "yTest", balance: 2100000000000000 },
        ],
        platformAddresses: [],
      });
      // 2100000000000000 duffs = 21000000.00000000 DASH
      expect(screen.getByText("21000000.00000000 DASH")).toBeInTheDocument();
    });
  });
});
