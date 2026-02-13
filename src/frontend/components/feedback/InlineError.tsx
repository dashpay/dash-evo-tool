import { AlertCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { translateError } from "@/lib/errorTranslation";

export interface InlineErrorProps {
  /** Raw error message from the backend. */
  message: string;
  /** Optional heading text (e.g. "Registration Failed"). */
  heading?: string;
  /** Called when the user dismisses the error. */
  onDismiss?: () => void;
  /** Label for the dismiss button. Defaults to "Dismiss". */
  dismissLabel?: string;
  /** Display as a full-screen centered layout instead of inline. */
  fullScreen?: boolean;
}

export function InlineError({
  message,
  heading,
  onDismiss,
  dismissLabel = "Dismiss",
  fullScreen = false,
}: InlineErrorProps) {
  const { summary, suggestion } = translateError(message);

  if (fullScreen) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <AlertCircle className="w-16 h-16 text-destructive" aria-hidden />
        {heading && <h2 className="text-xl font-semibold">{heading}</h2>}
        <div className="rounded-md border border-destructive/30 bg-destructive/5 p-4 max-w-md text-center space-y-2">
          <p className="text-sm text-destructive">{summary}</p>
          {suggestion && (
            <p className="text-sm italic text-muted-foreground">{suggestion}</p>
          )}
        </div>
        {onDismiss && (
          <Button variant="outline" onClick={onDismiss}>
            {dismissLabel}
          </Button>
        )}
      </div>
    );
  }

  return (
    <div
      className="rounded-md border border-destructive/30 bg-destructive/5 px-4 py-3 space-y-2"
      role="alert"
    >
      <div className="flex items-start gap-2">
        <AlertCircle className="h-4 w-4 text-destructive shrink-0 mt-0.5" />
        <div className="space-y-1">
          <p className="text-sm text-destructive">{summary}</p>
          {suggestion && (
            <p className="text-sm italic text-muted-foreground">{suggestion}</p>
          )}
        </div>
      </div>
      {onDismiss && (
        <Button variant="outline" size="sm" onClick={onDismiss}>
          {dismissLabel}
        </Button>
      )}
    </div>
  );
}
