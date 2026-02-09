/**
 * Translates raw backend error strings into user-friendly messages
 * with optional recovery suggestions.
 *
 * Port of src/ui/helpers.rs translate_backend_error() + recovery_suggestion()
 */

export interface TranslatedError {
  /** User-friendly summary message */
  summary: string;
  /** Technical details (original error string, if different from summary) */
  details: string;
  /** Actionable recovery suggestion, or empty string if none */
  suggestion: string;
}

/**
 * Translate a raw backend error string into a user-friendly summary
 * and technical details.
 */
export function translateBackendError(raw: string): { summary: string; details: string } {
  const lower = raw.toLowerCase();

  // Connection / transport errors
  if (lower.includes("transport(status")) {
    if (lower.includes("unavailable")) {
      return {
        summary: "Connection to Platform failed. Check your network connection.",
        details: raw,
      };
    }
    if (lower.includes("deadlineexceeded") || lower.includes("deadline exceeded")) {
      return {
        summary: "Request timed out. The Platform may be busy — try again later.",
        details: raw,
      };
    }
    if (lower.includes("internal")) {
      return {
        summary: "Platform returned an internal error. Try again later.",
        details: raw,
      };
    }
    return {
      summary: "Connection error while communicating with Platform.",
      details: raw,
    };
  }

  // Generic connection / network errors
  if (
    lower.includes("connection refused") ||
    lower.includes("connect error") ||
    lower.includes("connection reset")
  ) {
    return {
      summary:
        "Could not connect to the server. Check that Dash Core is running and your network connection is active.",
      details: raw,
    };
  }

  if (lower.includes("timed out") || lower.includes("timeout")) {
    return {
      summary: "The operation timed out. The server may be busy — try again later.",
      details: raw,
    };
  }

  // Insufficient funds / balance errors
  if (lower.includes("insufficient") && lower.includes("balance")) {
    return {
      summary: "Insufficient balance for this operation.",
      details: raw,
    };
  }
  if (lower.includes("insufficient")) {
    return {
      summary: "Insufficient funds for this operation.",
      details: raw,
    };
  }

  // Identity not found
  if (lower.includes("identity") && lower.includes("not found")) {
    return {
      summary: "The requested identity was not found on Platform.",
      details: raw,
    };
  }
  if (lower.includes("identity at revision")) {
    return {
      summary: "The identity could not be found at the expected revision.",
      details: raw,
    };
  }

  // Document / contract not found
  if (lower.includes("document type not found")) {
    return {
      summary: "The requested document type was not found in the contract.",
      details: raw,
    };
  }
  if (lower.includes("contract not found") || lower.includes("contractnotfound")) {
    return {
      summary: "The data contract was not found on Platform.",
      details: raw,
    };
  }
  if (lower.includes("notfound") || (lower.includes("not found") && lower.includes("query"))) {
    return {
      summary: "The requested item was not found on Platform.",
      details: raw,
    };
  }

  // Already exists
  if (lower.includes("already exists")) {
    return {
      summary: "This item already exists on Platform.",
      details: raw,
    };
  }

  // Invalid contract / protocol errors from DPP
  if (lower.includes("invalid contract") || lower.includes("protocolerror")) {
    return {
      summary: "The contract data is invalid or has been modified on Platform.",
      details: raw,
    };
  }

  // State transition broadcast errors
  if (
    lower.includes("state transition broadcast error") ||
    lower.includes("statetransition")
  ) {
    if (lower.includes("identity balance is not enough")) {
      return {
        summary: "Insufficient identity balance to cover this transaction's fees.",
        details: raw,
      };
    }
    return {
      summary: "Failed to broadcast the transaction to Platform.",
      details: raw,
    };
  }

  // RPC / Core errors
  if (lower.includes("wallet file not specified")) {
    return {
      summary:
        "Dash Core has multiple wallets loaded. Please ensure only one wallet is active.",
      details: raw,
    };
  }
  if (lower.includes("rpc") && lower.includes("error")) {
    return {
      summary:
        "Error communicating with Dash Core. Check that Core is running and configured correctly.",
      details: raw,
    };
  }

  // Authentication / cookie errors
  if (
    lower.includes("cookie") ||
    lower.includes("authentication") ||
    lower.includes("unauthorized")
  ) {
    return {
      summary:
        "Authentication with Dash Core failed. Check your RPC credentials or cookie file.",
      details: raw,
    };
  }

  // Database errors
  if (lower.includes("database") || lower.includes("rusqlite") || lower.includes("sqlite")) {
    return {
      summary: "A database error occurred. The application data may be corrupted.",
      details: raw,
    };
  }

  // Fee-related errors
  if (lower.includes("min relay fee not met") || lower.includes("fee rate")) {
    return {
      summary: "The transaction fee is too low. Try increasing the fee.",
      details: raw,
    };
  }

  // Consensus / validation errors
  if (lower.includes("consensus") && lower.includes("error")) {
    return {
      summary: "The transaction failed Platform validation.",
      details: raw,
    };
  }

  // Frozen token errors
  if (lower.includes("frozen")) {
    return {
      summary: "This token or identity is currently frozen.",
      details: raw,
    };
  }

  // Paused token errors
  if (lower.includes("paused")) {
    return {
      summary: "This token is currently paused.",
      details: raw,
    };
  }

  // If the raw error is already user-readable (short, no nested parens/braces),
  // use it directly as the summary.
  if (raw.length < 120 && !raw.includes("(Status") && !raw.includes("Protocol(")) {
    return { summary: raw, details: "" };
  }

  // Default: generic message with raw error as details
  return { summary: "Operation failed.", details: raw };
}

/**
 * Return a contextual recovery suggestion for a given error message.
 * Returns an empty string if no specific suggestion applies.
 */
export function recoverySuggestion(error: string): string {
  const lower = error.toLowerCase();

  // Connection / network errors
  if (
    lower.includes("connection") ||
    lower.includes("connect") ||
    lower.includes("unavailable") ||
    lower.includes("network")
  ) {
    return "Check your internet connection and ensure Dash Core is running, then try again.";
  }

  // Timeout errors
  if (lower.includes("timed out") || lower.includes("timeout") || lower.includes("deadline")) {
    return "The operation timed out. You can try again — the Platform may be busy.";
  }

  // Insufficient funds / balance
  if (lower.includes("insufficient") && lower.includes("balance")) {
    return "Verify you have sufficient funds and consider topping up your identity balance.";
  }
  if (lower.includes("insufficient")) {
    return "Verify you have sufficient funds and try again.";
  }

  // Fee errors
  if (lower.includes("fee") && (lower.includes("low") || lower.includes("not met"))) {
    return "Try increasing the fee and submitting again.";
  }

  // Key / authentication errors
  if (lower.includes("key") && (lower.includes("not found") || lower.includes("no eligible"))) {
    return "Ensure your wallet is unlocked and a valid key is selected.";
  }
  if (
    lower.includes("authentication") ||
    lower.includes("cookie") ||
    lower.includes("unauthorized")
  ) {
    return "Check your Dash Core RPC credentials or cookie file configuration.";
  }

  // Wallet-related
  if (lower.includes("wallet") && lower.includes("locked")) {
    return "Unlock your wallet and try again.";
  }
  if (lower.includes("wallet file not specified") || lower.includes("multiple wallets")) {
    return "Ensure only one wallet is loaded in Dash Core.";
  }

  // Identity not found
  if (lower.includes("identity") && lower.includes("not found")) {
    return "The identity may not exist on this network. Verify the identity ID and network.";
  }

  // Contract / document not found
  if (lower.includes("contract") && lower.includes("not found")) {
    return "Verify the contract ID is correct and exists on this network.";
  }
  if (lower.includes("not found")) {
    return "Verify the item exists on the current network and try again.";
  }

  // Already exists
  if (lower.includes("already exists")) {
    return "This item is already registered on Platform. No further action is needed.";
  }

  // Invalid contract / protocol
  if (lower.includes("invalid contract") || lower.includes("failed to load token contract")) {
    return "The contract may have been updated on Platform. Try refreshing your contracts.";
  }

  // Database errors
  if (lower.includes("database") || lower.includes("sqlite") || lower.includes("rusqlite")) {
    return "Try restarting the application. If the problem persists, the database may need to be reset.";
  }

  // Broadcast / state transition errors
  if (lower.includes("broadcast") || lower.includes("state transition")) {
    return "The transaction could not be broadcast. Check your connection and try again.";
  }

  // Frozen / paused token
  if (lower.includes("frozen")) {
    return "This token or identity is frozen. Contact the token issuer for assistance.";
  }
  if (lower.includes("paused")) {
    return "This token is currently paused. Wait for the token issuer to resume it.";
  }

  // Consensus / validation
  if (lower.includes("consensus") || lower.includes("validation")) {
    return "The transaction failed validation. Check your inputs and try again.";
  }

  // Platform internal error
  if (lower.includes("internal error") || lower.includes("platform returned")) {
    return "This is a Platform-side issue. Try again later.";
  }

  // No specific suggestion
  return "";
}

/**
 * Translate a raw backend error into a full TranslatedError with summary,
 * details, and recovery suggestion.
 */
export function translateError(raw: string): TranslatedError {
  const { summary, details } = translateBackendError(raw);
  const suggestion = recoverySuggestion(summary || raw);
  return { summary, details, suggestion };
}
