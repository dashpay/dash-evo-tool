import { Component } from "react";
import type { ErrorInfo, ReactNode } from "react";
import { AlertTriangle, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";

interface ErrorBoundaryProps {
  children: ReactNode;
  /** Optional fallback to render instead of the default error UI. */
  fallback?: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

/**
 * React Error Boundary that catches render errors in child components
 * and displays a user-friendly fallback UI with a reload button.
 *
 * Must be a class component — React does not support error boundaries
 * via hooks (getDerivedStateFromError / componentDidCatch).
 */
export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ErrorBoundary caught an error:", error, info.componentStack);
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null });
  };

  handleReload = () => {
    window.location.reload();
  };

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }

      return (
        <div
          className="flex h-full w-full flex-col items-center justify-center gap-4 p-8 text-center"
          role="alert"
        >
          <AlertTriangle className="h-12 w-12 text-destructive" aria-hidden="true" />
          <div className="space-y-1">
            <p className="text-lg font-medium text-foreground">
              Something went wrong
            </p>
            <p className="text-sm text-muted-foreground">
              An unexpected error occurred while rendering this page.
            </p>
          </div>
          {this.state.error && (
            <pre className="max-w-lg overflow-auto rounded-md bg-muted p-3 text-left text-xs text-muted-foreground">
              {this.state.error.message}
            </pre>
          )}
          <div className="flex gap-2">
            <Button variant="outline" onClick={this.handleReset}>
              <RotateCcw className="mr-2 h-4 w-4" />
              Try Again
            </Button>
            <Button onClick={this.handleReload}>Reload App</Button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
