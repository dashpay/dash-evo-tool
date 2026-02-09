import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { CopyButton } from "./CopyButton";

describe("CopyButton", () => {
  const writeTextMock = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    writeTextMock.mockClear();
    // Replace navigator.clipboard with a mock that our component can see
    Object.defineProperty(window.navigator, "clipboard", {
      value: { writeText: writeTextMock },
      writable: true,
      configurable: true,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders with copy-to-clipboard aria label", () => {
    render(<CopyButton value="test" />);
    expect(
      screen.getByRole("button", { name: "Copy to clipboard" }),
    ).toBeInTheDocument();
  });

  it("shows 'Copied' label after clicking", async () => {
    const user = userEvent.setup();
    render(<CopyButton value="test" />);
    await user.click(
      screen.getByRole("button", { name: "Copy to clipboard" }),
    );
    expect(
      screen.getByRole("button", { name: "Copied" }),
    ).toBeInTheDocument();
  });

  it("renders with custom label", () => {
    render(<CopyButton value="test" label="Copy ID" />);
    expect(screen.getByText("Copy ID")).toBeInTheDocument();
  });

  it("renders as ghost variant button", () => {
    render(<CopyButton value="test" />);
    const button = screen.getByRole("button");
    expect(button).toHaveAttribute("data-variant", "ghost");
  });

  it("supports custom className", () => {
    render(<CopyButton value="test" className="my-custom-class" />);
    expect(screen.getByRole("button")).toHaveClass("my-custom-class");
  });
});
