import { render, type RenderOptions } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";

/**
 * Renders a component wrapped in TooltipProvider (needed for Sidebar's tooltips).
 */
export function renderWithProviders(
  ui: React.ReactElement,
  options?: RenderOptions,
) {
  return render(ui, {
    wrapper: ({ children }) => (
      <TooltipProvider>{children}</TooltipProvider>
    ),
    ...options,
  });
}
