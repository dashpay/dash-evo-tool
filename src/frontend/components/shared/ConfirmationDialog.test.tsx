import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ConfirmationDialog } from "./ConfirmationDialog";

describe("ConfirmationDialog", () => {
  it("renders title and message when open", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Delete Wallet"
        message="Are you sure you want to delete this wallet?"
      />,
    );
    expect(screen.getByText("Delete Wallet")).toBeInTheDocument();
    expect(
      screen.getByText("Are you sure you want to delete this wallet?"),
    ).toBeInTheDocument();
  });

  it("does not render content when closed", () => {
    render(
      <ConfirmationDialog
        open={false}
        onOpenChange={() => {}}
        title="Delete Wallet"
        message="Are you sure?"
      />,
    );
    expect(screen.queryByText("Delete Wallet")).not.toBeInTheDocument();
  });

  it("shows default Confirm and Cancel buttons", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Test"
        message="Test message"
      />,
    );
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("uses custom button text", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Test"
        message="Test"
        confirmText="Yes, delete"
        cancelText="No, keep it"
      />,
    );
    expect(
      screen.getByRole("button", { name: "Yes, delete" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "No, keep it" }),
    ).toBeInTheDocument();
  });

  it("hides confirm button when confirmText is null", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Info"
        message="Just an info"
        confirmText={null}
      />,
    );
    expect(screen.queryByRole("button", { name: "Confirm" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("hides cancel button when cancelText is null", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Action"
        message="Proceed?"
        cancelText={null}
      />,
    );
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
  });

  it("calls onResult with 'confirmed' when confirm is clicked", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Test"
        message="Test"
        onResult={onResult}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onResult).toHaveBeenCalledWith("confirmed");
  });

  it("calls onResult with 'canceled' when cancel is clicked", async () => {
    const user = userEvent.setup();
    const onResult = vi.fn();
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Test"
        message="Test"
        onResult={onResult}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onResult).toHaveBeenCalledWith("canceled");
  });

  it("calls onOpenChange(false) when cancel is clicked", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={onOpenChange}
        title="Test"
        message="Test"
      />,
    );
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("renders destructive confirm button in danger mode", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Delete"
        message="This is permanent"
        danger={true}
      />,
    );
    const confirmBtn = screen.getByRole("button", { name: "Confirm" });
    expect(confirmBtn).toHaveAttribute("data-variant", "destructive");
  });

  it("renders default confirm button without danger mode", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Proceed"
        message="Continue?"
      />,
    );
    const confirmBtn = screen.getByRole("button", { name: "Confirm" });
    expect(confirmBtn).toHaveAttribute("data-variant", "default");
  });

  it("has accessible dialog role", () => {
    render(
      <ConfirmationDialog
        open={true}
        onOpenChange={() => {}}
        title="Test Dialog"
        message="Testing"
      />,
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
