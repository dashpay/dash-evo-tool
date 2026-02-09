import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WalletUnlockDialog } from "./WalletUnlockDialog";

describe("WalletUnlockDialog", () => {
  const defaultProps = {
    open: true,
    onOpenChange: vi.fn(),
    walletAlias: "My Wallet",
  };

  it("renders dialog with title and wallet alias", () => {
    render(<WalletUnlockDialog {...defaultProps} />);
    expect(screen.getByText("Unlock Wallet")).toBeInTheDocument();
    expect(
      screen.getByText(/Enter password to unlock/),
    ).toBeInTheDocument();
    expect(screen.getByText(/My Wallet/)).toBeInTheDocument();
  });

  it("does not render when closed", () => {
    render(<WalletUnlockDialog {...defaultProps} open={false} />);
    expect(screen.queryByText("Unlock Wallet")).not.toBeInTheDocument();
  });

  it("renders password input", () => {
    render(<WalletUnlockDialog {...defaultProps} />);
    const input = screen.getByPlaceholderText("Enter password");
    expect(input).toBeInTheDocument();
    expect(input).toHaveAttribute("type", "password");
  });

  it("renders Cancel and Unlock buttons", () => {
    render(<WalletUnlockDialog {...defaultProps} />);
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Unlock" })).toBeInTheDocument();
  });

  it("disables Unlock button when password is empty", () => {
    render(<WalletUnlockDialog {...defaultProps} />);
    expect(screen.getByRole("button", { name: "Unlock" })).toBeDisabled();
  });

  it("enables Unlock button when password is entered", async () => {
    const user = userEvent.setup();
    render(<WalletUnlockDialog {...defaultProps} />);
    await user.type(
      screen.getByPlaceholderText("Enter password"),
      "mypassword",
    );
    expect(screen.getByRole("button", { name: "Unlock" })).toBeEnabled();
  });

  it("calls onResult with password when Unlock is clicked", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(<WalletUnlockDialog {...defaultProps} onResult={onResult} />);
    await user.type(
      screen.getByPlaceholderText("Enter password"),
      "secret123",
    );
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    expect(onResult).toHaveBeenCalledWith({
      status: "unlocked",
      password: "secret123",
    });
  });

  it("calls onResult with cancelled when Cancel is clicked", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(<WalletUnlockDialog {...defaultProps} onResult={onResult} />);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onResult).toHaveBeenCalledWith({ status: "cancelled" });
  });

  it("toggles password visibility", async () => {
    const user = userEvent.setup();
    render(<WalletUnlockDialog {...defaultProps} />);
    const input = screen.getByPlaceholderText("Enter password");
    expect(input).toHaveAttribute("type", "password");

    await user.click(screen.getByLabelText("Show password"));
    expect(input).toHaveAttribute("type", "text");

    await user.click(screen.getByLabelText("Hide password"));
    expect(input).toHaveAttribute("type", "password");
  });

  it("displays error message", () => {
    render(
      <WalletUnlockDialog {...defaultProps} error="Incorrect password" />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Incorrect password");
  });

  it("displays error with password hint", () => {
    render(
      <WalletUnlockDialog
        {...defaultProps}
        error="Incorrect password"
        passwordHint="starts with 'a'"
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Incorrect password. Hint: starts with 'a'",
    );
  });

  it("marks password input as invalid when error exists", () => {
    render(
      <WalletUnlockDialog {...defaultProps} error="Incorrect password" />,
    );
    expect(screen.getByPlaceholderText("Enter password")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("shows loading state", () => {
    render(<WalletUnlockDialog {...defaultProps} loading={true} />);
    expect(screen.getByRole("button", { name: "Unlocking…" })).toBeDisabled();
    expect(screen.getByPlaceholderText("Enter password")).toBeDisabled();
  });

  it("submits on Enter key", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(<WalletUnlockDialog {...defaultProps} onResult={onResult} />);
    const input = screen.getByPlaceholderText("Enter password");
    await user.type(input, "secret123");
    await user.keyboard("{Enter}");
    expect(onResult).toHaveBeenCalledWith({
      status: "unlocked",
      password: "secret123",
    });
  });

  it("has accessible dialog role", () => {
    render(<WalletUnlockDialog {...defaultProps} />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
