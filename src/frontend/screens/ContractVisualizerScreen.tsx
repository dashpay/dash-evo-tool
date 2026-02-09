import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { HexInput, decodeToHex, detectFormat } from "@/components/tools/HexInput";
import { JsonViewer } from "@/components/shared/JsonViewer";
import { AlertCircle, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type ParseState =
  | { status: "idle" }
  | { status: "error"; message: string }
  | { status: "success"; json: string };

/**
 * Contract Visualizer tool screen.
 *
 * Accepts hex, base64, or comma-separated byte data representing a serialized
 * Dash DataContract. Parses on every change and displays the deserialized JSON
 * or an error message.
 */
export function ContractVisualizerScreen() {
  const [inputValue, setInputValue] = useState("");
  const [parseState, setParseState] = useState<ParseState>({ status: "idle" });
  const [isParsing, setIsParsing] = useState(false);

  // Debounce timer ref to avoid spamming the backend on every keystroke
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const parseInput = useCallback(async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) {
      setParseState({ status: "idle" });
      return;
    }

    const format = detectFormat(trimmed);
    const hexData = decodeToHex(trimmed, format);

    if (!hexData) {
      setParseState({
        status: "error",
        message: "Unable to decode input. Provide valid hex, base64, or comma-separated bytes (0-255).",
      });
      return;
    }

    setIsParsing(true);
    try {
      const result = await commands.parseDataContract({ hexData });
      if (result.status === "ok") {
        setParseState({ status: "success", json: result.data.json });
      } else {
        setParseState({ status: "error", message: result.error });
      }
    } catch (err) {
      setParseState({
        status: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setIsParsing(false);
    }
  }, []);

  // Debounced parse on input change
  const handleInputChange = useCallback(
    (value: string) => {
      setInputValue(value);
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
      debounceRef.current = setTimeout(() => {
        parseInput(value);
      }, 300);
    },
    [parseInput],
  );

  // Cleanup debounce timer on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
    };
  }, []);

  const dismissError = useCallback(() => {
    setParseState({ status: "idle" });
  }, []);

  return (
    <ToolPageLayout
      title="Contract Visualizer"
      subtitle="Deserialize and inspect Dash Platform data contracts"
    >
      <div className="flex flex-col gap-6">
        {/* Input section */}
        <HexInput
          value={inputValue}
          onChange={handleInputChange}
          label="Enter hex, base64, or comma-separated integers for Contract"
          placeholder="Paste serialized contract bytes here..."
          rows={5}
        />

        {/* Output section */}
        <div className="space-y-2">
          <span className="text-sm font-medium text-foreground">Result</span>

          {parseState.status === "idle" && !isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm italic text-muted-foreground">
                Awaiting input…
              </span>
            </div>
          )}

          {isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm text-muted-foreground">
                Parsing…
              </span>
            </div>
          )}

          {parseState.status === "error" && !isParsing && (
            <div
              className={cn(
                "flex items-start gap-3 rounded-md border border-destructive/50 bg-destructive/5 p-4",
              )}
              role="alert"
            >
              <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
              <div className="min-w-0 flex-1">
                <p className="text-sm text-destructive">{parseState.message}</p>
              </div>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={dismissError}
                aria-label="Dismiss error"
                className="shrink-0 text-destructive hover:text-destructive"
              >
                <X className="size-3.5" />
              </Button>
            </div>
          )}

          {parseState.status === "success" && !isParsing && (
            <JsonViewer
              data={parseState.json}
              expandDepth={4}
              className="max-h-[600px]"
            />
          )}
        </div>
      </div>
    </ToolPageLayout>
  );
}
