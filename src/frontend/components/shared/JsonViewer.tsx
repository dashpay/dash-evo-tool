import { JsonView, darkStyles, defaultStyles } from "react-json-view-lite";
import "react-json-view-lite/dist/index.css";
import { useTheme } from "@/components/theme/ThemeProvider";
import { CopyButton } from "./CopyButton";
import { cn } from "@/lib/utils";

interface JsonViewerProps {
  /** JSON data to display (object, array, string, etc.) */
  data: unknown;
  /** Whether nodes should be expanded by default */
  defaultExpanded?: boolean;
  /** Maximum depth to auto-expand (-1 = all) */
  expandDepth?: number;
  /** Show copy button to copy raw JSON */
  showCopy?: boolean;
  /** Additional CSS class */
  className?: string;
}

/**
 * Syntax-highlighted, collapsible JSON viewer with copy button.
 *
 * Uses react-json-view-lite for rendering with theme-aware styling.
 * Displays JSON data in a scrollable, collapsible tree view.
 */
export function JsonViewer({
  data,
  defaultExpanded = true,
  expandDepth = 3,
  showCopy = true,
  className,
}: JsonViewerProps) {
  const { resolvedTheme } = useTheme();
  const jsonString = typeof data === "string" ? data : JSON.stringify(data, null, 2);

  // Parse the data for the tree viewer
  let parsedData: unknown;
  try {
    parsedData = typeof data === "string" ? JSON.parse(data) : data;
  } catch {
    parsedData = data;
  }

  const shouldInitiallyExpand = (_level: number, _value: unknown, field?: string) => {
    // Always expand root
    if (field === "") return true;
    if (!defaultExpanded) return false;
    if (expandDepth < 0) return true;
    return _level < expandDepth;
  };

  return (
    <div
      className={cn(
        "relative rounded-md border bg-muted/30 text-sm font-mono overflow-auto",
        className,
      )}
    >
      {showCopy && (
        <div className="absolute top-2 right-2 z-10 flex gap-1">
          <CopyButton value={jsonString} />
        </div>
      )}
      <div className="p-3">
        {typeof parsedData === "object" && parsedData !== null ? (
          <JsonView
            data={parsedData as object}
            style={resolvedTheme === "dark" ? darkStyles : defaultStyles}
            shouldExpandNode={shouldInitiallyExpand}
          />
        ) : (
          <pre className="whitespace-pre-wrap break-all">{jsonString}</pre>
        )}
      </div>
    </div>
  );
}
