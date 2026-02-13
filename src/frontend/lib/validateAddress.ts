import { commands } from "@/bindings";

/**
 * Validate a Dash Core address against the current network via the Tauri backend.
 * Returns null if valid, or an error message string if invalid.
 */
export async function validateCoreAddress(
  address: string,
): Promise<string | null> {
  const trimmed = address.trim();
  if (!trimmed) return null;

  // Reject platform addresses up front (these are not Core addresses)
  if (trimmed.startsWith("evo1") || trimmed.startsWith("tevo1")) {
    return "Platform addresses are not supported. Use a Core address (e.g., X... or y...).";
  }

  const result = await commands.validateCoreAddress(trimmed);
  if (result.status === "error") {
    return result.error;
  }
  return null;
}
