/**
 * Parse a "min relay fee not met" error to extract the required fee.
 * Matches error format: "min relay fee not met, 226 < 1000"
 * Returns the required fee (the number after '<') or null if not a fee error.
 */
export function parseMinRelayFeeError(error: string): number | null {
  if (!error.includes("min relay fee")) return null;
  const ltPos = error.indexOf("<");
  if (ltPos === -1) return null;
  const afterLt = error.substring(ltPos + 1).trim();
  const digits = afterLt.match(/^(\d+)/);
  if (!digits) return null;
  const requiredFee = parseInt(digits[1] ?? "", 10);
  return isNaN(requiredFee) ? null : requiredFee;
}
