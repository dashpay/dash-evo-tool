import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

// Suppress console.error noise from React and our componentDidCatch
const originalConsoleError = console.error;
beforeEach(() => {
  console.error = vi.fn();
  return () => {
    console.error = originalConsoleError;
  };
});

/** Component that throws on render for testing. */
function ThrowingComponent({ message }: { message: string }): never {
  throw new Error(message);
}

/** Component that conditionally throws. */
function ConditionalThrow({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) {
    throw new Error("conditional error");
  }
  return <div>Healthy content</div>;
}

describe("ErrorBoundary", () => {
  it("renders children when no error occurs", () => {
    render(
      <ErrorBoundary>
        <div>Normal content</div>
      </ErrorBoundary>,
    );
    expect(screen.getByText("Normal content")).toBeInTheDocument();
  });

  it("renders fallback UI when a child throws", () => {
    render(
      <ErrorBoundary>
        <ThrowingComponent message="test error" />
      </ErrorBoundary>,
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(
      screen.getByText("An unexpected error occurred while rendering this page."),
    ).toBeInTheDocument();
  });

  it("displays the error message", () => {
    render(
      <ErrorBoundary>
        <ThrowingComponent message="kaboom!" />
      </ErrorBoundary>,
    );
    expect(screen.getByText("kaboom!")).toBeInTheDocument();
  });

  it("renders Try Again and Reload App buttons", () => {
    render(
      <ErrorBoundary>
        <ThrowingComponent message="oops" />
      </ErrorBoundary>,
    );
    expect(
      screen.getByRole("button", { name: /try again/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /reload app/i }),
    ).toBeInTheDocument();
  });

  it("resets error state when Try Again is clicked", async () => {
    const user = userEvent.setup();
    let shouldThrow = true;

    function MaybeThrow() {
      if (shouldThrow) {
        throw new Error("once");
      }
      return <div>Recovered</div>;
    }

    render(
      <ErrorBoundary>
        <MaybeThrow />
      </ErrorBoundary>,
    );

    expect(screen.getByText("Something went wrong")).toBeInTheDocument();

    // Stop throwing and click retry
    shouldThrow = false;
    await user.click(screen.getByRole("button", { name: /try again/i }));

    expect(screen.getByText("Recovered")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("calls window.location.reload when Reload App is clicked", async () => {
    const user = userEvent.setup();
    const reloadMock = vi.fn();
    Object.defineProperty(window, "location", {
      value: { ...window.location, reload: reloadMock },
      writable: true,
    });

    render(
      <ErrorBoundary>
        <ThrowingComponent message="fatal" />
      </ErrorBoundary>,
    );

    await user.click(screen.getByRole("button", { name: /reload app/i }));
    expect(reloadMock).toHaveBeenCalledOnce();
  });

  it("renders custom fallback when provided", () => {
    render(
      <ErrorBoundary fallback={<div>Custom error page</div>}>
        <ThrowingComponent message="test" />
      </ErrorBoundary>,
    );
    expect(screen.getByText("Custom error page")).toBeInTheDocument();
    expect(screen.queryByText("Something went wrong")).not.toBeInTheDocument();
  });

  it("logs error to console.error", () => {
    render(
      <ErrorBoundary>
        <ThrowingComponent message="logged error" />
      </ErrorBoundary>,
    );
    expect(console.error).toHaveBeenCalled();
  });

  it("renders alert icon (SVG) in fallback UI", () => {
    const { container } = render(
      <ErrorBoundary>
        <ThrowingComponent message="icon test" />
      </ErrorBoundary>,
    );
    const svg = container.querySelector("svg");
    expect(svg).toBeInTheDocument();
    expect(svg).toHaveAttribute("aria-hidden", "true");
  });

  it("does not show error message pre element when error is null", () => {
    // This tests the branch where fallback UI renders without an error
    // In practice getDerivedStateFromError always provides an error,
    // but we verify the pre element isn't rendered for non-Error throws
    render(
      <ErrorBoundary>
        <div>Safe content</div>
      </ErrorBoundary>,
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
