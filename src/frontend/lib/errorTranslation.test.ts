import { describe, it, expect } from "vitest";
import {
  translateBackendError,
  recoverySuggestion,
  translateError,
} from "./errorTranslation";

describe("translateBackendError", () => {
  describe("connection / transport errors", () => {
    it("translates unavailable transport status", () => {
      const raw =
        'Transport(Status { code: Unavailable, message: "failed to connect" })';
      const { summary, details } = translateBackendError(raw);
      expect(summary).toBe(
        "Connection to Platform failed. Check your network connection."
      );
      expect(details).toBe(raw);
    });

    it("translates deadline exceeded transport status", () => {
      const raw =
        'Transport(Status { code: DeadlineExceeded, message: "timeout" })';
      const { summary } = translateBackendError(raw);
      expect(summary).toContain("timed out");
    });

    it("translates internal transport error", () => {
      const raw =
        'Transport(Status { code: Internal, message: "server error" })';
      const { summary } = translateBackendError(raw);
      expect(summary).toContain("internal error");
    });

    it("translates generic transport error", () => {
      const raw =
        'Transport(Status { code: Unknown, message: "something" })';
      const { summary } = translateBackendError(raw);
      expect(summary).toContain("Connection error");
    });
  });

  describe("generic connection errors", () => {
    it("translates connection refused", () => {
      const { summary } = translateBackendError("Connection refused");
      expect(summary).toContain("Could not connect");
    });

    it("translates connect error", () => {
      const { summary } = translateBackendError("connect error: tls handshake failed");
      expect(summary).toContain("Could not connect");
    });

    it("translates connection reset", () => {
      const { summary } = translateBackendError("connection reset by peer");
      expect(summary).toContain("Could not connect");
    });
  });

  describe("timeout errors", () => {
    it("translates timed out", () => {
      const { summary } = translateBackendError("operation timed out");
      expect(summary).toContain("timed out");
    });

    it("translates timeout", () => {
      const { summary } = translateBackendError("request timeout after 30s");
      expect(summary).toContain("timed out");
    });
  });

  describe("insufficient funds errors", () => {
    it("translates insufficient balance", () => {
      const { summary } = translateBackendError(
        "Insufficient balance to complete transfer"
      );
      expect(summary).toBe("Insufficient balance for this operation.");
    });

    it("translates generic insufficient", () => {
      const { summary } = translateBackendError("Insufficient credits");
      expect(summary).toBe("Insufficient funds for this operation.");
    });
  });

  describe("identity errors", () => {
    it("translates identity not found", () => {
      const { summary } = translateBackendError(
        "Identity not found on platform"
      );
      expect(summary).toContain("identity was not found");
    });

    it("translates identity at revision", () => {
      const { summary } = translateBackendError(
        "Identity at revision 5 could not be located"
      );
      expect(summary).toContain("expected revision");
    });
  });

  describe("document / contract errors", () => {
    it("translates document type not found", () => {
      const { summary } = translateBackendError("Document type not found");
      expect(summary).toContain("document type was not found");
    });

    it("translates contract not found", () => {
      const { summary } = translateBackendError("Contract not found");
      expect(summary).toContain("data contract was not found");
    });

    it("translates contractnotfound (no spaces)", () => {
      const { summary } = translateBackendError(
        "ContractNotFound: abcdef"
      );
      expect(summary).toContain("data contract was not found");
    });

    it("translates generic not found with query context", () => {
      const { summary } = translateBackendError(
        "Document not found for query xyz"
      );
      expect(summary).toContain("not found on Platform");
    });
  });

  describe("already exists errors", () => {
    it("translates already exists", () => {
      const { summary } = translateBackendError(
        "DPNS name already exists"
      );
      expect(summary).toBe("This item already exists on Platform.");
    });
  });

  describe("invalid contract / protocol errors", () => {
    it("translates invalid contract", () => {
      const { summary } = translateBackendError(
        "Invalid contract definition"
      );
      expect(summary).toContain("invalid");
    });

    it("translates protocol error", () => {
      const { summary } = translateBackendError(
        "ProtocolError: version mismatch"
      );
      expect(summary).toContain("invalid");
    });
  });

  describe("state transition errors", () => {
    it("translates state transition broadcast error", () => {
      const { summary } = translateBackendError(
        "State transition broadcast error: rejected"
      );
      expect(summary).toContain("broadcast");
    });

    it("translates identity balance not enough in state transition", () => {
      const { summary } = translateBackendError(
        "StateTransition error: Identity balance is not enough"
      );
      expect(summary).toContain("Insufficient identity balance");
    });
  });

  describe("RPC / Core errors", () => {
    it("translates wallet file not specified", () => {
      const { summary } = translateBackendError(
        "Wallet file not specified"
      );
      expect(summary).toContain("multiple wallets");
    });

    it("translates RPC error", () => {
      const { summary } = translateBackendError(
        "RPC error: method not found"
      );
      expect(summary).toContain("Dash Core");
    });
  });

  describe("authentication errors", () => {
    it("translates cookie error", () => {
      const { summary } = translateBackendError(
        "Could not read cookie file"
      );
      expect(summary).toContain("Authentication");
    });

    it("translates unauthorized", () => {
      const { summary } = translateBackendError("Unauthorized access");
      expect(summary).toContain("Authentication");
    });
  });

  describe("database errors", () => {
    it("translates database error", () => {
      const { summary } = translateBackendError("Database write failed");
      expect(summary).toContain("database error");
    });

    it("translates rusqlite error", () => {
      const { summary } = translateBackendError(
        "rusqlite::Error: table not found"
      );
      expect(summary).toContain("database error");
    });
  });

  describe("fee errors", () => {
    it("translates min relay fee not met", () => {
      const { summary } = translateBackendError(
        "min relay fee not met"
      );
      expect(summary).toContain("fee is too low");
    });

    it("translates fee rate error", () => {
      const { summary } = translateBackendError(
        "fee rate below minimum"
      );
      expect(summary).toContain("fee is too low");
    });
  });

  describe("consensus errors", () => {
    it("translates consensus error", () => {
      const { summary } = translateBackendError(
        "Consensus error: invalid signature"
      );
      expect(summary).toContain("failed Platform validation");
    });
  });

  describe("token errors", () => {
    it("translates frozen token", () => {
      const { summary } = translateBackendError(
        "Token transfer rejected: identity is frozen"
      );
      expect(summary).toContain("frozen");
    });

    it("translates paused token", () => {
      const { summary } = translateBackendError(
        "Token is currently paused"
      );
      expect(summary).toContain("paused");
    });
  });

  describe("fallback behavior", () => {
    it("uses short raw error as summary when user-readable", () => {
      const { summary, details } = translateBackendError("Something went wrong");
      expect(summary).toBe("Something went wrong");
      expect(details).toBe("");
    });

    it("falls back to generic message for long/complex errors", () => {
      const raw =
        "Protocol(SomeComplexError { nested: { deep: true } }) with extra info " +
        "that makes this error very long and hard to read for a regular user " +
        "who just wants to know what happened";
      const { summary, details } = translateBackendError(raw);
      expect(summary).toBe("Operation failed.");
      expect(details).toBe(raw);
    });

    it("falls back for errors with (Status in them", () => {
      const raw = "Something (Status code 500) happened";
      const { summary, details } = translateBackendError(raw);
      expect(summary).toBe("Operation failed.");
      expect(details).toBe(raw);
    });
  });
});

describe("recoverySuggestion", () => {
  it("suggests checking connection for connection errors", () => {
    expect(recoverySuggestion("Connection to Platform failed")).toContain(
      "internet connection"
    );
  });

  it("suggests retry for timeout errors", () => {
    expect(recoverySuggestion("Request timed out")).toContain("try again");
  });

  it("suggests topping up for insufficient balance", () => {
    expect(recoverySuggestion("Insufficient balance")).toContain("topping up");
  });

  it("suggests verifying funds for generic insufficient", () => {
    expect(recoverySuggestion("Insufficient credits")).toContain(
      "sufficient funds"
    );
  });

  it("suggests increasing fee for fee errors", () => {
    expect(recoverySuggestion("The transaction fee is too low")).toContain(
      "increasing the fee"
    );
  });

  it("suggests unlocking wallet for key errors", () => {
    expect(recoverySuggestion("Key not found")).toContain("wallet is unlocked");
  });

  it("suggests checking credentials for auth errors", () => {
    expect(recoverySuggestion("Authentication failed")).toContain(
      "RPC credentials"
    );
  });

  it("suggests unlocking for locked wallet", () => {
    expect(recoverySuggestion("Wallet is locked")).toContain("Unlock");
  });

  it("suggests checking identity ID for identity not found", () => {
    expect(recoverySuggestion("Identity not found")).toContain(
      "identity ID"
    );
  });

  it("suggests verifying contract ID for contract not found", () => {
    expect(recoverySuggestion("Contract not found")).toContain("contract ID");
  });

  it("suggests no action for already exists", () => {
    expect(recoverySuggestion("Already exists")).toContain("No further action");
  });

  it("suggests refreshing for invalid contract", () => {
    expect(recoverySuggestion("Invalid contract")).toContain("refreshing");
  });

  it("suggests restarting for database errors", () => {
    expect(recoverySuggestion("Database error")).toContain("restarting");
  });

  it("suggests checking connection for broadcast errors", () => {
    expect(recoverySuggestion("Broadcast failed")).toContain("connection");
  });

  it("suggests contacting issuer for frozen token", () => {
    expect(recoverySuggestion("Token is frozen")).toContain("token issuer");
  });

  it("suggests waiting for paused token", () => {
    expect(recoverySuggestion("Token is paused")).toContain("Wait");
  });

  it("suggests checking inputs for validation errors", () => {
    expect(recoverySuggestion("Consensus validation failed")).toContain(
      "Check your inputs"
    );
  });

  it("suggests trying later for platform internal error", () => {
    expect(recoverySuggestion("Platform returned an internal error")).toContain(
      "Try again later"
    );
  });

  it("returns empty string for unknown errors", () => {
    expect(recoverySuggestion("xyzzy foobar baz")).toBe("");
  });
});

describe("translateError", () => {
  it("returns summary, details, and suggestion together", () => {
    const result = translateError(
      'Transport(Status { code: Unavailable, message: "failed" })'
    );
    expect(result.summary).toContain("Connection to Platform failed");
    expect(result.details).toContain("Transport");
    expect(result.suggestion).toContain("internet connection");
  });

  it("returns empty suggestion for unknown errors", () => {
    const result = translateError("something weird happened");
    expect(result.summary).toBe("something weird happened");
    expect(result.details).toBe("");
    expect(result.suggestion).toBe("");
  });

  it("provides suggestion based on summary, not raw error", () => {
    // The raw error contains "database" but is matched by "already exists" first
    const result = translateError("Username already exists in database");
    expect(result.summary).toBe("This item already exists on Platform.");
    expect(result.suggestion).toContain("No further action");
  });
});
