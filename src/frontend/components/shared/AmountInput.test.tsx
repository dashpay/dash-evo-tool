import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AmountInput, formatAmount } from "./AmountInput";

describe("AmountInput", () => {
  it("renders with label", () => {
    render(
      <AmountInput value="" onChange={() => {}} label="Amount" />,
    );
    expect(screen.getByText("Amount")).toBeInTheDocument();
  });

  it("renders input with placeholder", () => {
    render(
      <AmountInput
        value=""
        onChange={() => {}}
        placeholder="Enter amount"
      />,
    );
    expect(
      screen.getByPlaceholderText("Enter amount"),
    ).toBeInTheDocument();
  });

  it("displays the current value", () => {
    render(<AmountInput value="1.5" onChange={() => {}} />);
    expect(screen.getByRole("textbox")).toHaveValue("1.5");
  });

  it("calls onChange with new value", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<AmountInput value="" onChange={onChange} />);
    await user.type(screen.getByRole("textbox"), "1");
    expect(onChange).toHaveBeenCalledWith("1");
  });

  it("rejects non-numeric input", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<AmountInput value="" onChange={onChange} />);
    await user.type(screen.getByRole("textbox"), "abc");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("allows decimal input", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<AmountInput value="1" onChange={onChange} />);
    await user.type(screen.getByRole("textbox"), ".");
    expect(onChange).toHaveBeenCalledWith("1.");
  });

  it("shows Max button when enabled", () => {
    render(
      <AmountInput value="" onChange={() => {}} showMaxButton={true} />,
    );
    expect(
      screen.getByRole("button", { name: "Set maximum amount" }),
    ).toBeInTheDocument();
  });

  it("does not show Max button by default", () => {
    render(<AmountInput value="" onChange={() => {}} />);
    expect(
      screen.queryByRole("button", { name: "Set maximum amount" }),
    ).not.toBeInTheDocument();
  });

  it("calls onMaxClick when Max button is clicked", async () => {
    const user = userEvent.setup();
    const onMaxClick = vi.fn();
    render(
      <AmountInput
        value=""
        onChange={() => {}}
        showMaxButton={true}
        onMaxClick={onMaxClick}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Set maximum amount" }),
    );
    expect(onMaxClick).toHaveBeenCalled();
  });

  it("shows max exceeded error after blur", async () => {
    const user = userEvent.setup();
    render(
      <AmountInput
        value="10.0"
        onChange={() => {}}
        maxAmount={500000000}
        decimalPlaces={8}
        unitName="DASH"
      />,
    );
    const input = screen.getByRole("textbox");
    await user.click(input);
    await user.tab(); // blur
    expect(screen.getByRole("alert")).toHaveTextContent(
      /exceeds maximum/,
    );
  });

  it("shows max exceeded with hint", async () => {
    const user = userEvent.setup();
    render(
      <AmountInput
        value="10.0"
        onChange={() => {}}
        maxAmount={500000000}
        decimalPlaces={8}
        unitName="DASH"
        maxExceededHint="fees reserved"
      />,
    );
    const input = screen.getByRole("textbox");
    await user.click(input);
    await user.tab();
    expect(screen.getByRole("alert")).toHaveTextContent(/fees reserved/);
  });

  it("shows minimum amount error after blur", async () => {
    const user = userEvent.setup();
    render(
      <AmountInput
        value="0.00000000"
        onChange={() => {}}
        minAmount={1}
        decimalPlaces={8}
        unitName="DASH"
      />,
    );
    const input = screen.getByRole("textbox");
    await user.click(input);
    await user.tab();
    expect(screen.getByRole("alert")).toHaveTextContent(
      /Amount must be at least/,
    );
  });

  it("does not show error before blur", () => {
    render(
      <AmountInput
        value="999999"
        onChange={() => {}}
        maxAmount={100}
        decimalPlaces={0}
      />,
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("hides errors when showValidationErrors is false", async () => {
    const user = userEvent.setup();
    render(
      <AmountInput
        value="999999"
        onChange={() => {}}
        maxAmount={100}
        decimalPlaces={0}
        showValidationErrors={false}
      />,
    );
    const input = screen.getByRole("textbox");
    await user.click(input);
    await user.tab();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("marks input as aria-invalid when error exists", async () => {
    const user = userEvent.setup();
    render(
      <AmountInput
        value="999999"
        onChange={() => {}}
        maxAmount={100}
        decimalPlaces={0}
      />,
    );
    const input = screen.getByRole("textbox");
    await user.click(input);
    await user.tab();
    expect(input).toHaveAttribute("aria-invalid", "true");
  });

  it("disables the input when disabled prop is true", () => {
    render(
      <AmountInput value="" onChange={() => {}} disabled={true} />,
    );
    expect(screen.getByRole("textbox")).toBeDisabled();
  });

  it("has inputMode=decimal for mobile keyboards", () => {
    render(<AmountInput value="" onChange={() => {}} />);
    expect(screen.getByRole("textbox")).toHaveAttribute(
      "inputMode",
      "decimal",
    );
  });
});

describe("formatAmount", () => {
  it("formats amount with 8 decimals", () => {
    expect(formatAmount(150000000, 8)).toBe("1.50000000");
  });

  it("formats amount with 0 decimals", () => {
    expect(formatAmount(100, 0)).toBe("100");
  });

  it("formats zero", () => {
    expect(formatAmount(0, 8)).toBe("0.00000000");
  });

  it("formats amount with 2 decimals", () => {
    expect(formatAmount(1234, 2)).toBe("12.34");
  });
});
