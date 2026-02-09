import { toast } from "sonner";
import { translateError } from "./errorTranslation";

/**
 * Display a translated error toast with optional recovery suggestion.
 *
 * Usage:
 *   toastError(result.error)
 *   toastError(e instanceof Error ? e.message : String(e))
 *
 * This replaces direct `toast.error(rawMsg)` calls to provide
 * user-friendly summaries and actionable recovery suggestions.
 */
export function toastError(rawError: string): void {
  const { summary, suggestion } = translateError(rawError);
  const description = suggestion || undefined;
  toast.error(summary, { description });
}
