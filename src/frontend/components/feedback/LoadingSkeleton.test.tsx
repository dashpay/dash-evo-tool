import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { LoadingSkeleton, SkeletonGroup } from "./LoadingSkeleton";

describe("LoadingSkeleton", () => {
  it("renders with skeleton class", () => {
    const { container } = render(<LoadingSkeleton />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("skeleton");
  });

  it("applies default width and height", () => {
    const { container } = render(<LoadingSkeleton />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("w-full", "h-4");
  });

  it("accepts custom width and height", () => {
    const { container } = render(
      <LoadingSkeleton width="w-48" height="h-10" />,
    );
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("w-48", "h-10");
  });

  it("uses rounded-md by default", () => {
    const { container } = render(<LoadingSkeleton />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("rounded-md");
    expect(el).not.toHaveClass("rounded-full");
  });

  it("uses rounded-full when rounded prop is true", () => {
    const { container } = render(<LoadingSkeleton rounded />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("rounded-full");
  });

  it("has status role and loading label", () => {
    render(<LoadingSkeleton />);
    const el = screen.getByRole("status");
    expect(el).toHaveAttribute("aria-label", "Loading");
  });

  it("applies custom className", () => {
    const { container } = render(<LoadingSkeleton className="my-custom" />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("skeleton", "my-custom");
  });
});

describe("SkeletonGroup", () => {
  it("renders default 3 lines", () => {
    render(<SkeletonGroup />);
    const skeletons = screen.getAllByRole("status");
    // The parent div also has role="status", so total is 4 (1 group + 3 children)
    // Actually children have role="status" too — let's count skeleton class elements
    expect(skeletons).toHaveLength(4); // 1 group wrapper + 3 lines
  });

  it("renders custom number of lines", () => {
    const { container } = render(<SkeletonGroup lines={5} />);
    const skeletons = container.querySelectorAll(".skeleton");
    expect(skeletons).toHaveLength(5);
  });

  it("makes last line shorter (w-2/3)", () => {
    const { container } = render(<SkeletonGroup lines={3} />);
    const skeletons = container.querySelectorAll(".skeleton");
    expect(skeletons[0]).toHaveClass("w-full");
    expect(skeletons[1]).toHaveClass("w-full");
    expect(skeletons[2]).toHaveClass("w-2/3");
  });

  it("applies custom gap", () => {
    const { container } = render(<SkeletonGroup gap="gap-4" />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("gap-4");
  });

  it("has loading content aria label on wrapper", () => {
    const { container } = render(<SkeletonGroup />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveAttribute("aria-label", "Loading content");
  });

  it("applies custom className", () => {
    const { container } = render(<SkeletonGroup className="my-custom" />);
    const el = container.firstChild as HTMLElement;
    expect(el).toHaveClass("my-custom");
  });
});
