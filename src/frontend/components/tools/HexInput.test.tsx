import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { HexInput, detectFormat, decodeToHex } from "./HexInput";

describe("detectFormat", () => {
  it("detects hex strings", () => {
    expect(detectFormat("abcdef0123456789")).toBe("hex");
  });

  it("detects hex with 0x prefix", () => {
    expect(detectFormat("0xabcdef")).toBe("hex");
  });

  it("detects hex with spaces", () => {
    expect(detectFormat("ab cd ef 01")).toBe("hex");
  });

  it("detects hex with uppercase", () => {
    expect(detectFormat("ABCDEF")).toBe("hex");
  });

  it("detects base64 strings", () => {
    expect(detectFormat("SGVsbG8gV29ybGQ=")).toBe("base64");
  });

  it("detects CSV byte strings", () => {
    expect(detectFormat("72,101,108,108,111")).toBe("csv");
  });

  it("returns unknown for empty string", () => {
    expect(detectFormat("")).toBe("unknown");
  });

  it("returns unknown for whitespace only", () => {
    expect(detectFormat("   ")).toBe("unknown");
  });

  it("rejects odd-length hex", () => {
    // Odd-length should not be detected as hex
    expect(detectFormat("abc")).not.toBe("hex");
  });

  it("rejects CSV with values > 255", () => {
    expect(detectFormat("300,400,500")).not.toBe("csv");
  });
});

describe("decodeToHex", () => {
  it("decodes hex input to lowercase hex", () => {
    expect(decodeToHex("ABCDEF", "hex")).toBe("abcdef");
  });

  it("strips 0x prefix from hex", () => {
    expect(decodeToHex("0xABCD", "hex")).toBe("abcd");
  });

  it("strips spaces from hex", () => {
    expect(decodeToHex("AB CD EF", "hex")).toBe("abcdef");
  });

  it("decodes base64 to hex", () => {
    // "Hello" in base64 is "SGVsbG8="
    expect(decodeToHex("SGVsbG8=", "base64")).toBe("48656c6c6f");
  });

  it("decodes CSV to hex", () => {
    // 72=0x48, 101=0x65, 108=0x6c, 108=0x6c, 111=0x6f → "Hello"
    expect(decodeToHex("72,101,108,108,111", "csv")).toBe("48656c6c6f");
  });

  it("returns null for empty input", () => {
    expect(decodeToHex("", "hex")).toBeNull();
  });

  it("returns null for unknown format", () => {
    expect(decodeToHex("test", "unknown")).toBeNull();
  });

  it("returns null for invalid base64", () => {
    expect(decodeToHex("!!!invalid!!!", "base64")).toBeNull();
  });
});

describe("HexInput", () => {
  it("renders with a label", () => {
    render(<HexInput value="" onChange={vi.fn()} label="Input Data" />);
    expect(screen.getByText("Input Data")).toBeInTheDocument();
  });

  it("renders the textarea with placeholder", () => {
    render(
      <HexInput
        value=""
        onChange={vi.fn()}
        placeholder="Paste data here"
      />,
    );
    expect(screen.getByPlaceholderText("Paste data here")).toBeInTheDocument();
  });

  it("renders the current value", () => {
    render(<HexInput value="abcdef" onChange={vi.fn()} />);
    expect(screen.getByDisplayValue("abcdef")).toBeInTheDocument();
  });

  it("calls onChange when typing", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<HexInput value="" onChange={onChange} label="Data" />);
    const textarea = screen.getByLabelText("Data");
    await user.type(textarea, "ab");
    expect(onChange).toHaveBeenCalled();
  });

  it("shows Hex badge for hex input", () => {
    render(<HexInput value="abcdef" onChange={vi.fn()} />);
    expect(screen.getByText("Hex")).toBeInTheDocument();
  });

  it("shows Base64 badge for base64 input", () => {
    render(<HexInput value="SGVsbG8gV29ybGQ=" onChange={vi.fn()} />);
    expect(screen.getByText("Base64")).toBeInTheDocument();
  });

  it("shows CSV badge for CSV input", () => {
    render(<HexInput value="72,101,108" onChange={vi.fn()} />);
    expect(screen.getByText("CSV")).toBeInTheDocument();
  });

  it("does not show badge when input is empty", () => {
    render(<HexInput value="" onChange={vi.fn()} />);
    expect(screen.queryByText("Hex")).not.toBeInTheDocument();
    expect(screen.queryByText("Base64")).not.toBeInTheDocument();
    expect(screen.queryByText("CSV")).not.toBeInTheDocument();
  });

  it("displays error message", () => {
    render(
      <HexInput value="xyz" onChange={vi.fn()} error="Invalid format" />,
    );
    expect(screen.getByText("Invalid format")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("marks textarea as invalid when error is present", () => {
    render(
      <HexInput
        value="xyz"
        onChange={vi.fn()}
        label="Data"
        error="Bad input"
      />,
    );
    expect(screen.getByLabelText("Data")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("disables textarea when disabled prop is true", () => {
    render(<HexInput value="" onChange={vi.fn()} label="Data" disabled />);
    expect(screen.getByLabelText("Data")).toBeDisabled();
  });

  it("uses aria-label fallback when no label provided", () => {
    render(<HexInput value="" onChange={vi.fn()} />);
    expect(screen.getByLabelText("Byte data input")).toBeInTheDocument();
  });
});
