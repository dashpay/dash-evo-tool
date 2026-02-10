import { useCallback } from "react";
import { Plus, Minus, ChevronDown, ChevronRight, Info, AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

// ─── Types ──────────────────────────────────────────────────────────────────

export interface GroupMember {
  identityId: string;
  power: string;
}

export interface GroupConfig {
  requiredPower: string;
  members: GroupMember[];
  expanded: boolean;
}

export interface GroupsState {
  groups: GroupConfig[];
}

// ─── Defaults ───────────────────────────────────────────────────────────────

export function createDefaultGroupsState(): GroupsState {
  return {
    groups: [],
  };
}

function createDefaultGroup(): GroupConfig {
  return {
    requiredPower: "2",
    members: [
      { identityId: "", power: "1" },
      { identityId: "", power: "1" },
    ],
    expanded: true,
  };
}

// ─── Validation ─────────────────────────────────────────────────────────────

export interface GroupsValidation {
  valid: boolean;
  errors: Record<string, string>;
}

/** Simple Base58 check: alphanumeric, no 0/O/I/l, 32-44 chars */
function isValidBase58(s: string): boolean {
  return /^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]{32,44}$/.test(s);
}

export function validateGroups(state: GroupsState): GroupsValidation {
  const errors: Record<string, string> = {};

  for (let g = 0; g < state.groups.length; g++) {
    const group = state.groups[g];
    const prefix = `group_${g}`;

    // Required power
    const rp = group.requiredPower.trim();
    if (!rp) {
      errors[`${prefix}.requiredPower`] = "Required power is required";
    } else if (!/^\d+$/.test(rp)) {
      errors[`${prefix}.requiredPower`] = "Must be a positive integer";
    } else {
      const val = parseInt(rp, 10);
      if (val > 4294967295) {
        errors[`${prefix}.requiredPower`] = "Must be ≤ 4294967295 (u32)";
      }
    }

    // Members
    for (let m = 0; m < group.members.length; m++) {
      const member = group.members[m];
      const mPrefix = `${prefix}.member_${m}`;

      // Identity ID
      const id = member.identityId.trim();
      if (!id) {
        errors[`${mPrefix}.identityId`] = "Identity ID is required";
      } else if (!isValidBase58(id)) {
        errors[`${mPrefix}.identityId`] = "Invalid Base58 identity";
      }

      // Power
      const pow = member.power.trim();
      if (!pow) {
        errors[`${mPrefix}.power`] = "Power is required";
      } else if (!/^\d+$/.test(pow)) {
        errors[`${mPrefix}.power`] = "Must be a positive integer";
      } else {
        const val = parseInt(pow, 10);
        if (val > 4294967295) {
          errors[`${mPrefix}.power`] = "Must be ≤ 4294967295 (u32)";
        }
      }
    }

    // Duplicate identity check within group
    const seen = new Set<string>();
    for (let m = 0; m < group.members.length; m++) {
      const id = group.members[m].identityId.trim();
      if (id && seen.has(id)) {
        errors[`${prefix}.member_${m}.duplicate`] = "Duplicate identity in this group";
      }
      if (id) seen.add(id);
    }
  }

  return {
    valid: Object.keys(errors).length === 0,
    errors,
  };
}

// ─── InfoTooltip ────────────────────────────────────────────────────────────

function InfoTooltip({ text }: { text: string }) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="inline-flex items-center justify-center rounded-full p-0.5 text-muted-foreground hover:text-foreground transition-colors"
            aria-label="More information"
          >
            <Info className="h-3.5 w-3.5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right" className="max-w-xs whitespace-pre-line">
          {text}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

// ─── GroupsStep ─────────────────────────────────────────────────────────────

export interface GroupsStepProps {
  state: GroupsState;
  onChange: (state: GroupsState) => void;
}

export function GroupsStep({ state, onChange }: GroupsStepProps) {
  const validation = validateGroups(state);

  const addGroup = useCallback(() => {
    onChange({
      ...state,
      groups: [...state.groups, createDefaultGroup()],
    });
  }, [state, onChange]);

  const removeGroup = useCallback(
    (index: number) => {
      onChange({
        ...state,
        groups: state.groups.filter((_, i) => i !== index),
      });
    },
    [state, onChange],
  );

  const updateGroup = useCallback(
    (index: number, update: Partial<GroupConfig>) => {
      const newGroups = [...state.groups];
      newGroups[index] = { ...newGroups[index], ...update };
      onChange({ ...state, groups: newGroups });
    },
    [state, onChange],
  );

  const toggleGroupExpanded = useCallback(
    (index: number) => {
      const newGroups = [...state.groups];
      newGroups[index] = { ...newGroups[index], expanded: !newGroups[index].expanded };
      onChange({ ...state, groups: newGroups });
    },
    [state, onChange],
  );

  const addMember = useCallback(
    (groupIndex: number) => {
      const newGroups = [...state.groups];
      newGroups[groupIndex] = {
        ...newGroups[groupIndex],
        members: [
          ...newGroups[groupIndex].members,
          { identityId: "", power: "1" },
        ],
      };
      onChange({ ...state, groups: newGroups });
    },
    [state, onChange],
  );

  const removeMember = useCallback(
    (groupIndex: number, memberIndex: number) => {
      const newGroups = [...state.groups];
      newGroups[groupIndex] = {
        ...newGroups[groupIndex],
        members: newGroups[groupIndex].members.filter((_, i) => i !== memberIndex),
      };
      onChange({ ...state, groups: newGroups });
    },
    [state, onChange],
  );

  const updateMember = useCallback(
    (groupIndex: number, memberIndex: number, update: Partial<GroupMember>) => {
      const newGroups = [...state.groups];
      const newMembers = [...newGroups[groupIndex].members];
      newMembers[memberIndex] = { ...newMembers[memberIndex], ...update };
      newGroups[groupIndex] = { ...newGroups[groupIndex], members: newMembers };
      onChange({ ...state, groups: newGroups });
    },
    [state, onChange],
  );

  return (
    <div className="space-y-4" data-testid="groups-step">
      <p className="text-sm text-muted-foreground">
        Define one or more groups for multi-party control of the contract.
        Groups allow multiple identities to collectively authorize token actions.
      </p>

      {state.groups.length === 0 && (
        <div
          className="rounded-lg border border-dashed p-6 text-center text-muted-foreground"
          data-testid="groups-empty-state"
        >
          <p className="text-sm">No groups defined yet.</p>
          <p className="text-xs mt-1">
            Groups are optional. Add a group if you want multi-party authorization
            for token operations.
          </p>
        </div>
      )}

      {state.groups.map((group, groupIndex) => (
        <div
          key={groupIndex}
          className="rounded-lg border bg-card"
          data-testid={`group-${groupIndex}`}
        >
          {/* Group header */}
          <button
            type="button"
            className="flex w-full items-center justify-between px-4 py-3 hover:bg-muted/50 transition-colors"
            onClick={() => toggleGroupExpanded(groupIndex)}
            data-testid={`group-${groupIndex}-toggle`}
          >
            <div className="flex items-center gap-2">
              {group.expanded ? (
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
              <span className="font-medium text-sm">Group {groupIndex}</span>
              {!group.expanded && (
                <span className="text-xs text-muted-foreground ml-2">
                  {group.members.length} member{group.members.length !== 1 ? "s" : ""}, required power: {group.requiredPower || "—"}
                </span>
              )}
            </div>
          </button>

          {/* Group content */}
          {group.expanded && (
            <div className="px-4 pb-4 space-y-4 border-t">
              {/* Required Power */}
              <div className="space-y-1 pt-3">
                <div className="flex items-center gap-1">
                  <Label htmlFor={`group-${groupIndex}-required-power`}>
                    Required Power
                  </Label>
                  <InfoTooltip text="The minimum combined power required from group members to authorize an action. Must be an unsigned 32-bit integer." />
                </div>
                <Input
                  id={`group-${groupIndex}-required-power`}
                  data-testid={`group-${groupIndex}-required-power`}
                  value={group.requiredPower}
                  onChange={(e) => {
                    const val = e.target.value.replace(/[^0-9]/g, "");
                    updateGroup(groupIndex, { requiredPower: val });
                  }}
                  placeholder="e.g. 2"
                  className="w-32"
                  inputMode="numeric"
                />
                {validation.errors[`group_${groupIndex}.requiredPower`] && (
                  <p className="text-xs text-destructive">
                    {validation.errors[`group_${groupIndex}.requiredPower`]}
                  </p>
                )}
              </div>

              {/* Members */}
              <div className="space-y-3">
                <Label>Members</Label>
                {group.members.map((member, memberIndex) => {
                  const mPrefix = `group_${groupIndex}.member_${memberIndex}`;
                  const idValid = member.identityId.trim() && isValidBase58(member.identityId.trim());
                  const idInvalid = member.identityId.trim() && !isValidBase58(member.identityId.trim());
                  return (
                    <div key={memberIndex} className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="text-xs text-muted-foreground w-20 shrink-0">
                          Member {memberIndex + 1}:
                        </span>
                        <div className="flex-1 relative">
                          <Input
                            data-testid={`group-${groupIndex}-member-${memberIndex}-identity`}
                            value={member.identityId}
                            onChange={(e) =>
                              updateMember(groupIndex, memberIndex, { identityId: e.target.value })
                            }
                            placeholder="Base58 Identity ID"
                            className={cn(
                              "font-mono text-xs pr-8",
                              idValid && "border-green-500/50",
                              idInvalid && "border-destructive/50",
                            )}
                          />
                          {idValid && (
                            <span className="absolute right-2 top-1/2 -translate-y-1/2 text-green-500 text-xs" data-testid={`group-${groupIndex}-member-${memberIndex}-valid`}>
                              ✓
                            </span>
                          )}
                        </div>
                        <div className="w-24 shrink-0">
                          <Input
                            data-testid={`group-${groupIndex}-member-${memberIndex}-power`}
                            value={member.power}
                            onChange={(e) => {
                              const val = e.target.value.replace(/[^0-9]/g, "");
                              updateMember(groupIndex, memberIndex, { power: val });
                            }}
                            placeholder="Power"
                            inputMode="numeric"
                          />
                        </div>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => removeMember(groupIndex, memberIndex)}
                          disabled={group.members.length <= 1}
                          title="Remove member"
                          data-testid={`group-${groupIndex}-member-${memberIndex}-remove`}
                          className="shrink-0"
                        >
                          <Minus className="h-4 w-4" />
                        </Button>
                      </div>
                      {validation.errors[`${mPrefix}.identityId`] && (
                        <p className="text-xs text-destructive pl-20">
                          {validation.errors[`${mPrefix}.identityId`]}
                        </p>
                      )}
                      {validation.errors[`${mPrefix}.power`] && (
                        <p className="text-xs text-destructive pl-20">
                          {validation.errors[`${mPrefix}.power`]}
                        </p>
                      )}
                      {validation.errors[`${mPrefix}.duplicate`] && (
                        <div className="flex items-center gap-1 pl-20">
                          <AlertTriangle className="h-3 w-3 text-amber-500" />
                          <p className="text-xs text-amber-500">
                            {validation.errors[`${mPrefix}.duplicate`]}
                          </p>
                        </div>
                      )}
                    </div>
                  );
                })}
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => addMember(groupIndex)}
                  data-testid={`group-${groupIndex}-add-member`}
                >
                  <Plus className="h-3.5 w-3.5 mr-1" />
                  Add Member
                </Button>
              </div>

              {/* Remove group */}
              {groupIndex === state.groups.length - 1 && (
                <div className="pt-2 border-t">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="text-destructive hover:text-destructive"
                    onClick={() => removeGroup(groupIndex)}
                    data-testid={`group-${groupIndex}-remove`}
                  >
                    <Minus className="h-3.5 w-3.5 mr-1" />
                    Remove This Group
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      ))}

      <Button
        variant="outline"
        onClick={addGroup}
        data-testid="add-group"
      >
        <Plus className="h-4 w-4 mr-1" />
        Add New Group
      </Button>
    </div>
  );
}
