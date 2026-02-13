/**
 * Client-side fee estimation for Platform operations.
 * Mirrors constants from src/fee_estimation.rs (PlatformFeeEstimator).
 * All values are in Platform credits.
 */

const DOCUMENT_BATCH_SUB_TRANSITION_MIN_FEE = 100_000;

/** Estimate fee for a document batch transition (credits). */
export function estimateDocumentBatch(count: number): number {
  return count * DOCUMENT_BATCH_SUB_TRANSITION_MIN_FEE;
}

/** Estimate fee for a token transition (credits). */
export function estimateTokenTransition(): number {
  // base + (100 processing units * 27_000) + (100 storage units * 400) + (8 signatures * 2_000)
  return 100_000 + 100 * 27_000 + 100 * 400 + 8 * 2_000;
}
