import { cn } from "@/lib/utils";

interface LoadingSkeletonProps {
  /** Width class (e.g. "w-full", "w-48"). Defaults to "w-full". */
  width?: string;
  /** Height class (e.g. "h-4", "h-10"). Defaults to "h-4". */
  height?: string;
  /** Use rounded-full instead of rounded-md (for avatars/circles). */
  rounded?: boolean;
  className?: string;
}

/**
 * Shimmer-effect skeleton loader for content that is still loading.
 *
 * Uses the `.skeleton` CSS class defined in index.css which provides
 * the shimmer gradient animation.
 */
export function LoadingSkeleton({
  width = "w-full",
  height = "h-4",
  rounded = false,
  className,
}: LoadingSkeletonProps) {
  return (
    <div
      className={cn(
        "skeleton",
        width,
        height,
        rounded ? "rounded-full" : "rounded-md",
        className,
      )}
      role="status"
      aria-label="Loading"
    />
  );
}

interface SkeletonGroupProps {
  /** Number of skeleton lines to render. */
  lines?: number;
  /** Gap between lines. Defaults to "gap-2". */
  gap?: string;
  className?: string;
}

/**
 * Renders a group of skeleton lines, useful for loading text blocks.
 * The last line is half-width for a natural appearance.
 */
export function SkeletonGroup({
  lines = 3,
  gap = "gap-2",
  className,
}: SkeletonGroupProps) {
  return (
    <div className={cn("flex flex-col", gap, className)} role="status" aria-label="Loading content">
      {Array.from({ length: lines }, (_, i) => (
        <LoadingSkeleton
          key={i}
          width={i === lines - 1 ? "w-2/3" : "w-full"}
        />
      ))}
    </div>
  );
}
