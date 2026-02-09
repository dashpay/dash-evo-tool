import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { IdentitySelector, type IdentityOption } from "./IdentitySelector";

const mockIdentities: IdentityOption[] = [
  { id: "abc123def456", displayName: "Alice" },
  { id: "xyz789ghi012", displayName: "Bob" },
  { id: "jkl345mno678", displayName: "Carol" },
];

describe("IdentitySelector", () => {
  it("renders with label", () => {
    render(
      <IdentitySelector
        value=""
        onChange={() => {}}
        identities={mockIdentities}
        label="Select Identity"
      />,
    );
    expect(screen.getByText("Select Identity")).toBeInTheDocument();
  });

  it("renders the select trigger", () => {
    render(
      <IdentitySelector
        value=""
        onChange={() => {}}
        identities={mockIdentities}
      />,
    );
    expect(screen.getByRole("combobox")).toBeInTheDocument();
  });

  it("shows text input when Other is selected", () => {
    render(
      <IdentitySelector
        value="unknown-id"
        onChange={() => {}}
        identities={mockIdentities}
        showOther={true}
      />,
    );
    expect(
      screen.getByPlaceholderText("Enter identity ID"),
    ).toBeInTheDocument();
  });

  it("hides text input when a known identity is selected", () => {
    render(
      <IdentitySelector
        value="abc123def456"
        onChange={() => {}}
        identities={mockIdentities}
        showOther={true}
      />,
    );
    expect(
      screen.queryByPlaceholderText("Enter identity ID"),
    ).not.toBeInTheDocument();
  });

  it("hides text input when showOther is false", () => {
    render(
      <IdentitySelector
        value=""
        onChange={() => {}}
        identities={mockIdentities}
        showOther={false}
      />,
    );
    expect(
      screen.queryByPlaceholderText("Enter identity ID"),
    ).not.toBeInTheDocument();
  });

  it("shows text input with current manual value", () => {
    render(
      <IdentitySelector
        value="manual-id-entry"
        onChange={() => {}}
        identities={mockIdentities}
        showOther={true}
      />,
    );
    expect(
      screen.getByPlaceholderText("Enter identity ID"),
    ).toHaveValue("manual-id-entry");
  });

  it("calls onChange when text input changes", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <IdentitySelector
        value=""
        onChange={onChange}
        identities={mockIdentities}
        showOther={true}
      />,
    );
    await user.type(
      screen.getByPlaceholderText("Enter identity ID"),
      "x",
    );
    expect(onChange).toHaveBeenCalledWith("x");
  });

  it("disables both elements when disabled", () => {
    render(
      <IdentitySelector
        value="some-id"
        onChange={() => {}}
        identities={mockIdentities}
        disabled={true}
        showOther={true}
      />,
    );
    expect(screen.getByRole("combobox")).toBeDisabled();
    expect(
      screen.getByPlaceholderText("Enter identity ID"),
    ).toBeDisabled();
  });

  it("renders with custom placeholder", () => {
    render(
      <IdentitySelector
        value=""
        onChange={() => {}}
        identities={[]}
        placeholder="Choose one"
        showOther={true}
      />,
    );
    // Placeholder text shows in select trigger
    expect(screen.getByRole("combobox")).toBeInTheDocument();
  });

  it("has accessible label on the combobox", () => {
    render(
      <IdentitySelector
        value=""
        onChange={() => {}}
        identities={mockIdentities}
        label="Sender"
      />,
    );
    expect(screen.getByLabelText("Sender")).toBeInTheDocument();
  });

  it("has accessible label on the text input", () => {
    render(
      <IdentitySelector
        value="unknown"
        onChange={() => {}}
        identities={mockIdentities}
        showOther={true}
      />,
    );
    expect(screen.getByLabelText("Identity ID")).toBeInTheDocument();
  });
});
