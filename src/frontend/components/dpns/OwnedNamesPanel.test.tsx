import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { OwnedNamesPanel } from "./OwnedNamesPanel";
import type { OwnedNamesPanelProps } from "./OwnedNamesPanel";
import type { DpnsNameEntryDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ────────────────────────────────────────────────

function makeName(
  overrides: Partial<DpnsNameEntryDto> = {},
): DpnsNameEntryDto {
  return {
    identityId: "abcdef1234567890abcdef1234567890",
    name: "alice.dash",
    acquiredAt: Math.floor(Date.now() / 1000) - 86400, // 1 day ago
    ...overrides,
  };
}

const defaultProps: OwnedNamesPanelProps = {
  ownedNames: [makeName()],
  filterTerm: "",
  onFilterChange: vi.fn(),
  onSetAlias: vi.fn(),
};

function setup(props: Partial<OwnedNamesPanelProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <OwnedNamesPanel {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Empty state ──────────────────────────────────────────────────

describe("OwnedNamesPanel — empty state", () => {
  it("shows empty state when no owned names", () => {
    setup({ ownedNames: [] });
    expect(screen.getByText("No owned usernames.")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Register a DPNS name to see it here. Names owned by your loaded identities will appear in this list.",
      ),
    ).toBeInTheDocument();
  });

  it("shows 'no matching' message when filter matches nothing", () => {
    setup({ filterTerm: "zzzznonexistent" });
    expect(screen.getByText("No matching names.")).toBeInTheDocument();
  });

  it("empty state has role=status", () => {
    setup({ ownedNames: [] });
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("no-match empty state has role=status", () => {
    setup({ filterTerm: "zzzznotfound" });
    expect(screen.getByRole("status")).toBeInTheDocument();
  });
});

// ─── Table rendering ──────────────────────────────────────────────

describe("OwnedNamesPanel — rendering", () => {
  it("renders the name in the table", () => {
    setup();
    expect(screen.getByText("alice.dash")).toBeInTheDocument();
  });

  it("renders the truncated owner ID", () => {
    setup();
    expect(screen.getByText("abcdef12...")).toBeInTheDocument();
  });

  it("renders relative acquired at time", () => {
    setup();
    expect(screen.getByText("1d ago")).toBeInTheDocument();
  });

  it("renders multiple owned names", () => {
    setup({
      ownedNames: [
        makeName({ name: "alice.dash" }),
        makeName({
          name: "bob.dash",
          identityId: "11112222333344445555666677778888",
        }),
      ],
    });
    expect(screen.getByText("alice.dash")).toBeInTheDocument();
    expect(screen.getByText("bob.dash")).toBeInTheDocument();
  });

  it("renders Set Alias button for each name", () => {
    setup({
      ownedNames: [
        makeName({ name: "alice.dash" }),
        makeName({ name: "bob.dash" }),
      ],
    });
    const aliasButtons = screen.getAllByRole("button", {
      name: /set alias for/i,
    });
    expect(aliasButtons).toHaveLength(2);
  });

  it("renders recent acquisition as 'just now'", () => {
    setup({
      ownedNames: [
        makeName({ acquiredAt: Math.floor(Date.now() / 1000) - 5 }),
      ],
    });
    expect(screen.getByText("just now")).toBeInTheDocument();
  });

  it("renders hours-old acquisition correctly", () => {
    setup({
      ownedNames: [
        makeName({ acquiredAt: Math.floor(Date.now() / 1000) - 7200 }),
      ],
    });
    expect(screen.getByText("2h ago")).toBeInTheDocument();
  });
});

// ─── Filter ───────────────────────────────────────────────────────

describe("OwnedNamesPanel — filtering", () => {
  it("renders the filter input", () => {
    setup();
    expect(screen.getByLabelText("Filter owned names")).toBeInTheDocument();
  });

  it("calls onFilterChange when typing", async () => {
    const { user, props } = setup();
    const input = screen.getByLabelText("Filter owned names");
    await user.type(input, "a");
    expect(props.onFilterChange).toHaveBeenCalled();
  });

  it("shows filter input value", () => {
    setup({ filterTerm: "test" });
    const input = screen.getByLabelText(
      "Filter owned names",
    ) as HTMLInputElement;
    expect(input.value).toBe("test");
  });

  it("filters names by substring match", () => {
    setup({
      ownedNames: [
        makeName({ name: "alice.dash" }),
        makeName({ name: "bob.dash" }),
      ],
      filterTerm: "alice",
    });
    expect(screen.getByText("alice.dash")).toBeInTheDocument();
    expect(screen.queryByText("bob.dash")).not.toBeInTheDocument();
  });

  it("filter is case-insensitive", () => {
    setup({
      ownedNames: [makeName({ name: "Alice.dash" })],
      filterTerm: "alice",
    });
    expect(screen.getByText("Alice.dash")).toBeInTheDocument();
  });
});

// ─── Sorting ──────────────────────────────────────────────────────

describe("OwnedNamesPanel — sorting", () => {
  it("renders all sortable column headers", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /sort by name/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by owner id/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /sort by acquired at/i }),
    ).toBeInTheDocument();
  });

  it("renders Actions column header (non-sortable)", () => {
    setup();
    expect(screen.getByText("Actions")).toBeInTheDocument();
  });

  it("sorts by name ascending by default", () => {
    setup({
      ownedNames: [
        makeName({ name: "zorro.dash" }),
        makeName({ name: "alice.dash" }),
      ],
    });
    const rows = screen.getAllByRole("row");
    // row 0 is header, row 1 should be alice, row 2 should be zorro
    expect(rows[1]).toHaveTextContent("alice.dash");
    expect(rows[2]).toHaveTextContent("zorro.dash");
  });

  it("toggles sort order when clicking same column", async () => {
    const { user } = setup({
      ownedNames: [
        makeName({ name: "alice.dash" }),
        makeName({ name: "zorro.dash" }),
      ],
    });

    // Click name sort to toggle to descending
    await user.click(screen.getByRole("button", { name: /sort by name/i }));

    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("zorro.dash");
    expect(rows[2]).toHaveTextContent("alice.dash");
  });

  it("sorts by acquired at when clicking that header", async () => {
    const { user } = setup({
      ownedNames: [
        makeName({
          name: "old.dash",
          acquiredAt: Math.floor(Date.now() / 1000) - 172800,
        }), // 2 days ago
        makeName({
          name: "new.dash",
          acquiredAt: Math.floor(Date.now() / 1000) - 3600,
        }), // 1 hour ago
      ],
    });

    await user.click(
      screen.getByRole("button", { name: /sort by acquired at/i }),
    );

    const rows = screen.getAllByRole("row");
    // Ascending: oldest first
    expect(rows[1]).toHaveTextContent("old.dash");
    expect(rows[2]).toHaveTextContent("new.dash");
  });

  it("sorts by owner ID when clicking that header", async () => {
    const { user } = setup({
      ownedNames: [
        makeName({
          name: "alice.dash",
          identityId: "zzzzzzzz0000000000000000",
        }),
        makeName({
          name: "bob.dash",
          identityId: "aaaaaaaa0000000000000000",
        }),
      ],
    });

    await user.click(
      screen.getByRole("button", { name: /sort by owner id/i }),
    );

    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("bob.dash");
    expect(rows[2]).toHaveTextContent("alice.dash");
  });
});

// ─── Set Alias action ────────────────────────────────────────────

describe("OwnedNamesPanel — Set Alias", () => {
  it("calls onSetAlias when clicking Set Alias button", async () => {
    const onSetAlias = vi.fn();
    const { user } = setup({
      ownedNames: [makeName()],
      onSetAlias,
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for alice.dash/i }),
    );

    expect(onSetAlias).toHaveBeenCalledWith(
      "abcdef1234567890abcdef1234567890",
      "alice.dash",
    );
  });

  it("calls onSetAlias with correct identity for each row", async () => {
    const onSetAlias = vi.fn();
    const { user } = setup({
      ownedNames: [
        makeName({ name: "alice.dash", identityId: "id-alice" }),
        makeName({ name: "bob.dash", identityId: "id-bob" }),
      ],
      onSetAlias,
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for bob.dash/i }),
    );

    expect(onSetAlias).toHaveBeenCalledWith("id-bob", "bob.dash");
    expect(onSetAlias).toHaveBeenCalledTimes(1);
  });

  it("Set Alias button has accessible label", () => {
    setup({
      ownedNames: [makeName({ name: "test.dash" })],
    });
    expect(
      screen.getByRole("button", { name: /set alias for test.dash/i }),
    ).toBeInTheDocument();
  });
});

// ─── Accessibility ───────────────────────────────────────────────

describe("OwnedNamesPanel — accessibility", () => {
  it("filter input has accessible label", () => {
    setup();
    expect(screen.getByLabelText("Filter owned names")).toBeInTheDocument();
  });

  it("sort buttons have aria-label", () => {
    setup();
    const sortButtons = screen.getAllByRole("button", {
      name: /sort by/i,
    });
    expect(sortButtons.length).toBe(3); // Name, Owner ID, Acquired At
  });

  it("Set Alias buttons have aria-labels with name", () => {
    setup({
      ownedNames: [
        makeName({ name: "alice.dash" }),
        makeName({ name: "bob.dash" }),
      ],
    });
    expect(
      screen.getByRole("button", { name: /set alias for alice.dash/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /set alias for bob.dash/i }),
    ).toBeInTheDocument();
  });
});
