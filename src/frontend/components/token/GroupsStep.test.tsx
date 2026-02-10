import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  GroupsStep,
  createDefaultGroupsState,
  validateGroups,
} from "./GroupsStep";
import type { GroupsState } from "./GroupsStep";

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── Helpers ────────────────────────────────────────────────────────

const VALID_ID_1 = "6YfbFRQBzMmQL8a8JCHEQomFHPjsfbMVMpBjmpcivjMH";
const VALID_ID_2 = "3GU1EJE3CPzHnMfdBPvfnN5f1uFNxJmfKHTCu7g8Lk8s";

function stateWithOneGroup(overrides?: Partial<GroupsState>): GroupsState {
  return {
    groups: [
      {
        requiredPower: "2",
        members: [
          { identityId: VALID_ID_1, power: "1" },
          { identityId: VALID_ID_2, power: "1" },
        ],
        expanded: true,
      },
    ],
    ...overrides,
  };
}

// ─── createDefaultGroupsState ───────────────────────────────────────

describe("createDefaultGroupsState", () => {
  it("returns empty groups array", () => {
    const state = createDefaultGroupsState();
    expect(state.groups).toHaveLength(0);
  });
});

// ─── validateGroups ─────────────────────────────────────────────────

describe("validateGroups", () => {
  it("validates empty state (no groups) as valid", () => {
    const result = validateGroups(createDefaultGroupsState());
    expect(result.valid).toBe(true);
  });

  it("validates a well-formed group", () => {
    const result = validateGroups(stateWithOneGroup());
    expect(result.valid).toBe(true);
  });

  it("rejects empty required power", () => {
    const state = stateWithOneGroup();
    state.groups[0].requiredPower = "";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.requiredPower"]).toBe("Required power is required");
  });

  it("rejects non-numeric required power", () => {
    const state = stateWithOneGroup();
    state.groups[0].requiredPower = "abc";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.requiredPower"]).toContain("positive integer");
  });

  it("rejects required power > u32 max", () => {
    const state = stateWithOneGroup();
    state.groups[0].requiredPower = "4294967296";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.requiredPower"]).toContain("4294967295");
  });

  it("rejects empty member identity", () => {
    const state = stateWithOneGroup();
    state.groups[0].members[0].identityId = "";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.member_0.identityId"]).toBe("Identity ID is required");
  });

  it("rejects invalid Base58 identity", () => {
    const state = stateWithOneGroup();
    state.groups[0].members[0].identityId = "INVALID0OIl!@#";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.member_0.identityId"]).toContain("Invalid Base58");
  });

  it("rejects empty member power", () => {
    const state = stateWithOneGroup();
    state.groups[0].members[0].power = "";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.member_0.power"]).toBe("Power is required");
  });

  it("rejects non-numeric member power", () => {
    const state = stateWithOneGroup();
    state.groups[0].members[0].power = "xyz";
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_0.member_0.power"]).toContain("positive integer");
  });

  it("detects duplicate identities within a group", () => {
    const state = stateWithOneGroup();
    state.groups[0].members[1].identityId = VALID_ID_1; // same as member 0
    const result = validateGroups(state);
    expect(result.errors["group_0.member_1.duplicate"]).toContain("Duplicate");
  });

  it("validates multiple groups independently", () => {
    const state: GroupsState = {
      groups: [
        {
          requiredPower: "2",
          members: [{ identityId: VALID_ID_1, power: "1" }],
          expanded: true,
        },
        {
          requiredPower: "",
          members: [{ identityId: VALID_ID_2, power: "1" }],
          expanded: true,
        },
      ],
    };
    const result = validateGroups(state);
    expect(result.valid).toBe(false);
    expect(result.errors["group_1.requiredPower"]).toBeDefined();
    expect(result.errors["group_0.requiredPower"]).toBeUndefined();
  });
});

// ─── GroupsStep rendering ───────────────────────────────────────────

describe("GroupsStep", () => {
  let onChange: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onChange = vi.fn();
  });

  it("renders the step container", () => {
    render(<GroupsStep state={createDefaultGroupsState()} onChange={onChange} />);
    expect(screen.getByTestId("groups-step")).toBeInTheDocument();
  });

  it("shows description text", () => {
    render(<GroupsStep state={createDefaultGroupsState()} onChange={onChange} />);
    expect(screen.getByText(/Define one or more groups/)).toBeInTheDocument();
  });

  it("shows empty state when no groups", () => {
    render(<GroupsStep state={createDefaultGroupsState()} onChange={onChange} />);
    expect(screen.getByTestId("groups-empty-state")).toBeInTheDocument();
    expect(screen.getByText(/No groups defined/)).toBeInTheDocument();
  });

  it("shows Add New Group button", () => {
    render(<GroupsStep state={createDefaultGroupsState()} onChange={onChange} />);
    expect(screen.getByTestId("add-group")).toBeInTheDocument();
  });

  it("adds a group when Add New Group is clicked", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={createDefaultGroupsState()} onChange={onChange} />);
    await user.click(screen.getByTestId("add-group"));
    const call = onChange.mock.calls[0][0] as GroupsState;
    expect(call.groups).toHaveLength(1);
    expect(call.groups[0].requiredPower).toBe("2");
    expect(call.groups[0].members).toHaveLength(2);
    expect(call.groups[0].expanded).toBe(true);
  });

  it("renders group card when a group exists", () => {
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    expect(screen.getByTestId("group-0")).toBeInTheDocument();
    expect(screen.getByText("Group 0")).toBeInTheDocument();
  });

  it("shows required power input for expanded group", () => {
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    expect(screen.getByTestId("group-0-required-power")).toHaveValue("2");
  });

  it("shows member identity and power inputs", () => {
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    expect(screen.getByTestId("group-0-member-0-identity")).toHaveValue(VALID_ID_1);
    expect(screen.getByTestId("group-0-member-0-power")).toHaveValue("1");
    expect(screen.getByTestId("group-0-member-1-identity")).toHaveValue(VALID_ID_2);
    expect(screen.getByTestId("group-0-member-1-power")).toHaveValue("1");
  });

  it("shows valid indicator for valid identity", () => {
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    expect(screen.getByTestId("group-0-member-0-valid")).toBeInTheDocument();
  });

  it("shows Add Member button", () => {
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    expect(screen.getByTestId("group-0-add-member")).toBeInTheDocument();
  });

  it("adds a member when Add Member is clicked", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    await user.click(screen.getByTestId("group-0-add-member"));
    const call = onChange.mock.calls[0][0] as GroupsState;
    expect(call.groups[0].members).toHaveLength(3);
    expect(call.groups[0].members[2].power).toBe("1");
  });

  it("removes a member when Remove is clicked", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    await user.click(screen.getByTestId("group-0-member-1-remove"));
    const call = onChange.mock.calls[0][0] as GroupsState;
    expect(call.groups[0].members).toHaveLength(1);
    expect(call.groups[0].members[0].identityId).toBe(VALID_ID_1);
  });

  it("disables Remove when only one member remains", () => {
    const state: GroupsState = {
      groups: [
        {
          requiredPower: "2",
          members: [{ identityId: VALID_ID_1, power: "1" }],
          expanded: true,
        },
      ],
    };
    render(<GroupsStep state={state} onChange={onChange} />);
    expect(screen.getByTestId("group-0-member-0-remove")).toBeDisabled();
  });

  it("shows Remove This Group button for last group only", () => {
    const state: GroupsState = {
      groups: [
        {
          requiredPower: "2",
          members: [{ identityId: VALID_ID_1, power: "1" }],
          expanded: true,
        },
        {
          requiredPower: "3",
          members: [{ identityId: VALID_ID_2, power: "1" }],
          expanded: true,
        },
      ],
    };
    render(<GroupsStep state={state} onChange={onChange} />);
    // Only last group has remove button
    expect(screen.queryByTestId("group-0-remove")).not.toBeInTheDocument();
    expect(screen.getByTestId("group-1-remove")).toBeInTheDocument();
  });

  it("removes a group when Remove This Group is clicked", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    await user.click(screen.getByTestId("group-0-remove"));
    const call = onChange.mock.calls[0][0] as GroupsState;
    expect(call.groups).toHaveLength(0);
  });

  it("collapses group when toggle is clicked", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    await user.click(screen.getByTestId("group-0-toggle"));
    const call = onChange.mock.calls[0][0] as GroupsState;
    expect(call.groups[0].expanded).toBe(false);
  });

  it("hides group content when collapsed", () => {
    const state = stateWithOneGroup();
    state.groups[0].expanded = false;
    render(<GroupsStep state={state} onChange={onChange} />);
    expect(screen.queryByTestId("group-0-required-power")).not.toBeInTheDocument();
    // Collapsed header shows member count
    expect(screen.getByText(/2 members/)).toBeInTheDocument();
  });

  it("updates required power on input", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    const input = screen.getByTestId("group-0-required-power");
    await user.type(input, "5");
    // Appends "5" to existing "2" → "25"
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0] as GroupsState;
    expect(lastCall.groups[0].requiredPower).toBe("25");
  });

  it("strips non-numeric from required power", async () => {
    const user = userEvent.setup();
    render(<GroupsStep state={stateWithOneGroup()} onChange={onChange} />);
    const input = screen.getByTestId("group-0-required-power");
    await user.type(input, "abc3");
    // "abc" stripped, only "3" appended to existing "2" → "23"
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1][0] as GroupsState;
    expect(lastCall.groups[0].requiredPower).toBe("23");
  });

  it("updates member identity on input", async () => {
    const user = userEvent.setup();
    const state: GroupsState = {
      groups: [
        {
          requiredPower: "2",
          members: [{ identityId: "", power: "1" }],
          expanded: true,
        },
      ],
    };
    render(<GroupsStep state={state} onChange={onChange} />);
    await user.type(screen.getByTestId("group-0-member-0-identity"), "abc");
    expect(onChange).toHaveBeenCalled();
  });

  it("shows validation error for invalid identity", () => {
    const state: GroupsState = {
      groups: [
        {
          requiredPower: "2",
          members: [{ identityId: "bad!", power: "1" }],
          expanded: true,
        },
      ],
    };
    render(<GroupsStep state={state} onChange={onChange} />);
    expect(screen.getByText("Invalid Base58 identity")).toBeInTheDocument();
  });

  it("shows duplicate warning", () => {
    const state: GroupsState = {
      groups: [
        {
          requiredPower: "2",
          members: [
            { identityId: VALID_ID_1, power: "1" },
            { identityId: VALID_ID_1, power: "1" },
          ],
          expanded: true,
        },
      ],
    };
    render(<GroupsStep state={state} onChange={onChange} />);
    expect(screen.getByText(/Duplicate identity/)).toBeInTheDocument();
  });
});
