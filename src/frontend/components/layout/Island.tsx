import { cn } from "@/lib/utils";

interface IslandProps {
  children: React.ReactNode;
  className?: string;
  /** HTML element to render. Defaults to "div". */
  as?: "div" | "section" | "article" | "aside" | "nav";
  /** Remove default padding (e.g. when content handles its own padding). */
  noPadding?: boolean;
}

/**
 * Elevated card surface used for main content panels.
 * Matches the egui "island" pattern: surface bg, rounded-lg border, elevated shadow.
 *
 * Design spec from 2.1 META:
 * - Surface background (white light / dark-gray dark)
 * - Rounded corners (radius-lg = 16px)
 * - Subtle border
 * - Elevated shadow
 * - Default padding: 24px (lg)
 */
export function Island({
  children,
  className,
  as: Component = "div",
  noPadding = false,
}: IslandProps) {
  return (
    <Component
      className={cn(
        "island",
        !noPadding && "p-6",
        className,
      )}
    >
      {children}
    </Component>
  );
}
