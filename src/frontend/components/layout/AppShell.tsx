import { cn } from "@/lib/utils";

interface AppShellProps {
  sidebar: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}

/**
 * Root layout container: fixed sidebar on the left, scrollable content area on the right.
 * Matches the egui three-panel "island" design from left_panel.rs + top_panel.rs.
 *
 * Sidebar is fixed-width (72px collapsed, 200px expanded via sidebar content).
 * Content area fills remaining space with its own scroll context.
 */
export function AppShell({ sidebar, children, className }: AppShellProps) {
  return (
    <div
      className={cn(
        "flex h-screen w-screen overflow-hidden bg-background",
        className,
      )}
    >
      {/* Sidebar region — fixed width, full height, no scroll at this level */}
      <aside className="flex-shrink-0" role="navigation" aria-label="Main navigation">
        {sidebar}
      </aside>

      {/* Content region — fills remaining width, manages its own scrolling */}
      <main className="flex min-w-0 flex-1 flex-col overflow-hidden" role="main">
        {children}
      </main>
    </div>
  );
}
