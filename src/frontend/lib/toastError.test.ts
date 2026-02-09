import { describe, it, expect, vi, beforeEach } from "vitest";
import { toast } from "sonner";
import { toastError } from "./toastError";

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
  },
}));

describe("toastError", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows translated summary as toast title", () => {
    toastError('Transport(Status { code: Unavailable, message: "down" })');
    expect(toast.error).toHaveBeenCalledWith(
      "Connection to Platform failed. Check your network connection.",
      { description: "Check your internet connection and ensure Dash Core is running, then try again." }
    );
  });

  it("shows recovery suggestion as toast description", () => {
    toastError("Insufficient balance for transfer");
    expect(toast.error).toHaveBeenCalledWith(
      "Insufficient balance for this operation.",
      { description: "Verify you have sufficient funds and consider topping up your identity balance." }
    );
  });

  it("omits description when no suggestion available", () => {
    toastError("something weird");
    expect(toast.error).toHaveBeenCalledWith("something weird", {
      description: undefined,
    });
  });

  it("handles short user-readable errors", () => {
    toastError("File not found");
    expect(toast.error).toHaveBeenCalledWith("File not found", expect.any(Object));
  });
});
