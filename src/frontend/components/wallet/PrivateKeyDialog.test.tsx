import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { PrivateKeyDialog } from "./PrivateKeyDialog";

const TEST_ADDRESS = "XjE7SUMkG3MWYtDdLx6vNMRw2MBL4t4Vet";
const TEST_WIF = "cRkVTJqfMPTEH8Zf8kD8jYoLQxHGzRzGDZsFGxYvoJRCGpDUF9MQ";
const MASKED_KEY = "••••••••••••••••••••••••••••••••••••••••••••••••••••";

function renderDialog(overrides?: Partial<React.ComponentProps<typeof PrivateKeyDialog>>) {
  const defaultProps = {
    open: true,
    onOpenChange: vi.fn(),
    address: TEST_ADDRESS,
    privateKeyWif: TEST_WIF,
    ...overrides,
  };
  return { ...render(<PrivateKeyDialog {...defaultProps} />), props: defaultProps };
}

describe("PrivateKeyDialog", () => {
  beforeEach(() => {
    // Provide clipboard mock for CopyButton
    Object.defineProperty(window.navigator, "clipboard", {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      writable: true,
      configurable: true,
    });
  });

  // ── Rendering ──────────────────────────────────────────────────────

  it("renders dialog with title", () => {
    renderDialog();
    expect(screen.getByRole("heading", { name: "Private Key" })).toBeInTheDocument();
  });

  it("does not render when closed", () => {
    renderDialog({ open: false });
    expect(screen.queryByRole("heading", { name: "Private Key" })).not.toBeInTheDocument();
  });

  it("closes on Escape key", async () => {
    const user = userEvent.setup();
    const { props } = renderDialog();
    await user.keyboard("{Escape}");
    expect(props.onOpenChange).toHaveBeenCalledWith(false);
  });

  // ── Address Section ────────────────────────────────────────────────

  it("displays the address label", () => {
    renderDialog();
    expect(screen.getByText("Address")).toBeInTheDocument();
  });

  it("displays the address in monospace code block", () => {
    renderDialog();
    const codeBlocks = screen.getAllByText(TEST_ADDRESS);
    expect(codeBlocks.length).toBeGreaterThan(0);
    // The first match should be in a <code> element
    expect(codeBlocks[0].tagName).toBe("CODE");
  });

  it("has a copy button for the address", () => {
    renderDialog();
    // There should be a copy button with "Copy" label for the address
    const copyButtons = screen.getAllByText("Copy");
    expect(copyButtons.length).toBeGreaterThanOrEqual(1);
  });

  // ── Private Key Section ────────────────────────────────────────────

  it("displays 'Private Key (WIF)' label", () => {
    renderDialog();
    expect(screen.getByText("Private Key (WIF)")).toBeInTheDocument();
  });

  it("shows masked key by default", () => {
    renderDialog();
    expect(screen.getByText(MASKED_KEY)).toBeInTheDocument();
    expect(screen.queryByText(TEST_WIF)).not.toBeInTheDocument();
  });

  it("has aria-label indicating key is hidden by default", () => {
    renderDialog();
    expect(screen.getByLabelText("Private key hidden")).toBeInTheDocument();
  });

  it("does not show copy key button when key is hidden", () => {
    renderDialog();
    // Only 1 copy button (for address) when key is hidden
    const copyButtons = screen.getAllByText("Copy");
    expect(copyButtons).toHaveLength(1);
  });

  // ── Show/Hide Toggle ──────────────────────────────────────────────

  it("reveals key when 'Show Key' button is clicked", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.click(screen.getByRole("button", { name: "Show private key" }));

    expect(screen.getByText(TEST_WIF)).toBeInTheDocument();
    expect(screen.queryByText(MASKED_KEY)).not.toBeInTheDocument();
  });

  it("changes button text to 'Hide Key' when revealed", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.click(screen.getByRole("button", { name: "Show private key" }));

    expect(screen.getByText("Hide Key")).toBeInTheDocument();
    expect(screen.queryByText("Show Key")).not.toBeInTheDocument();
  });

  it("has correct aria-label when key is revealed", async () => {
    const user = userEvent.setup();
    renderDialog();

    await user.click(screen.getByRole("button", { name: "Show private key" }));

    expect(screen.getByLabelText("Private key revealed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide private key" })).toBeInTheDocument();
  });

  it("hides key when 'Hide Key' button is clicked", async () => {
    const user = userEvent.setup();
    renderDialog();

    // Reveal
    await user.click(screen.getByRole("button", { name: "Show private key" }));
    expect(screen.getByText(TEST_WIF)).toBeInTheDocument();

    // Hide again
    await user.click(screen.getByRole("button", { name: "Hide private key" }));
    expect(screen.getByText(MASKED_KEY)).toBeInTheDocument();
    expect(screen.queryByText(TEST_WIF)).not.toBeInTheDocument();
  });

  // ── Copy Functionality ─────────────────────────────────────────────

  it("has copy button for address that can be clicked", async () => {
    const user = userEvent.setup();
    renderDialog();

    // The Copy button next to the address should exist
    const copyButtons = screen.getAllByRole("button", { name: "Copy to clipboard" });
    expect(copyButtons).toHaveLength(1);

    // Clicking it should change to "Copied" feedback
    await user.click(copyButtons[0]);
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
    });
  });

  it("has copy button for key when revealed", async () => {
    const user = userEvent.setup();
    renderDialog();

    // Reveal key first
    await user.click(screen.getByRole("button", { name: "Show private key" }));

    // Should have 2 copy buttons (address + key)
    const copyButtons = screen.getAllByRole("button", { name: "Copy to clipboard" });
    expect(copyButtons).toHaveLength(2);
  });

  // ── Security Warning ──────────────────────────────────────────────

  it("displays security warning", () => {
    renderDialog();
    expect(
      screen.getByText("Keep your private key secure. Never share it with anyone."),
    ).toBeInTheDocument();
  });

  it("security warning has alert role", () => {
    renderDialog();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Keep your private key secure. Never share it with anyone.",
    );
  });

  // ── Close Button ──────────────────────────────────────────────────

  it("has a Close button in footer", () => {
    renderDialog();
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("calls onOpenChange(false) when Close is clicked", async () => {
    const user = userEvent.setup();
    const { props } = renderDialog();

    await user.click(screen.getByRole("button", { name: "Close" }));

    expect(props.onOpenChange).toHaveBeenCalledWith(false);
  });

  // ── State Reset on Close ──────────────────────────────────────────

  it("resets key visibility when dialog closes and reopens", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    const { rerender } = render(
      <PrivateKeyDialog
        open={true}
        onOpenChange={onOpenChange}
        address={TEST_ADDRESS}
        privateKeyWif={TEST_WIF}
      />,
    );

    // Reveal key
    await user.click(screen.getByRole("button", { name: "Show private key" }));
    expect(screen.getByText(TEST_WIF)).toBeInTheDocument();

    // Close dialog
    rerender(
      <PrivateKeyDialog
        open={false}
        onOpenChange={onOpenChange}
        address={TEST_ADDRESS}
        privateKeyWif={TEST_WIF}
      />,
    );

    // Reopen dialog
    rerender(
      <PrivateKeyDialog
        open={true}
        onOpenChange={onOpenChange}
        address={TEST_ADDRESS}
        privateKeyWif={TEST_WIF}
      />,
    );

    // Should be masked again
    expect(screen.getByText(MASKED_KEY)).toBeInTheDocument();
    expect(screen.queryByText(TEST_WIF)).not.toBeInTheDocument();
  });

  // ── Accessibility ─────────────────────────────────────────────────

  it("has accessible dialog structure", () => {
    renderDialog();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("has sr-only description", () => {
    renderDialog();
    expect(
      screen.getByText("View the private key for this address"),
    ).toBeInTheDocument();
  });

  // ── Edge Cases ─────────────────────────────────────────────────────

  it("handles long addresses gracefully", () => {
    const longAddr = "X".repeat(100);
    renderDialog({ address: longAddr });
    expect(screen.getByText(longAddr)).toBeInTheDocument();
  });

  it("handles long WIF keys gracefully", async () => {
    const user = userEvent.setup();
    const longKey = "c".repeat(100);
    renderDialog({ privateKeyWif: longKey });

    await user.click(screen.getByRole("button", { name: "Show private key" }));
    expect(screen.getByText(longKey)).toBeInTheDocument();
  });
});
