import { useState, useMemo, useCallback } from "react";
import { Check, AlertCircle, Loader2, Clock, Vote } from "lucide-react";
import { cn, displayId } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import type { QualifiedIdentityDto, VoteChoiceDto } from "@/bindings";
import type { SelectedVote } from "@/stores/contestStore";

// ─── Types ─────────────────────────────────────────────────────────

/** Per-identity vote method. */
export type VoteMethod = "noVote" | "castNow" | "schedule";

/** Per-identity vote option with optional schedule time. */
export interface IdentityVoteOption {
  identityId: string;
  method: VoteMethod;
  scheduleDays: number;
  scheduleHours: number;
  scheduleMinutes: number;
}

/** Status of the bulk vote operation. */
export type VoteHandlingStatus =
  | { type: "notStarted" }
  | { type: "casting"; startTime: number }
  | { type: "scheduling" }
  | { type: "completed"; results: VoteResultSummary }
  | { type: "failed"; message: string };

/** Summary of vote results after execution. */
export interface VoteResultSummary {
  totalCast: number;
  totalScheduled: number;
  castSucceeded: number;
  castFailed: number;
  scheduleSucceeded: number;
  scheduleFailed: number;
  errors: string[];
}

export interface VoteCastingDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selectedVotes: SelectedVote[];
  votingIdentities: QualifiedIdentityDto[];
  onApplyVotes: (
    immediateIdentityIds: string[],
    scheduledVotes: Array<{
      voterId: string;
      unixTimestamp: number;
    }>,
  ) => Promise<void>;
  onGoToActiveContests?: () => void;
  onGoToScheduledVotes?: () => void;
  votingInProgress?: boolean;
}

// ─── Helpers ───────────────────────────────────────────────────────

/** Format a VoteChoiceDto to a human-readable string. */
export function formatVoteChoice(choice: VoteChoiceDto): string {
  if (choice === "lock") return "Lock";
  if (choice === "abstain") return "Abstain";
  if (typeof choice === "object" && "towardsIdentity" in choice) {
    return displayId(choice.towardsIdentity.identityId);
  }
  return "Unknown";
}

/** Format a timestamp as relative time. */
function formatRelativeTime(timestamp: number): string {
  const now = Date.now();
  const diff = timestamp - now;
  if (diff <= 0) return "expired";
  const days = Math.floor(diff / 86_400_000);
  const hours = Math.floor((diff % 86_400_000) / 3_600_000);
  const minutes = Math.floor((diff % 3_600_000) / 60_000);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  return parts.length > 0 ? `in ${parts.join(" ")}` : "< 1m";
}

/** Format elapsed time since a start timestamp. */
function formatElapsed(startTime: number): string {
  const elapsed = Math.max(0, Date.now() - startTime);
  const seconds = Math.floor(elapsed / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainSec = seconds % 60;
  return `${minutes}m ${remainSec}s`;
}

/** Get display name for an identity. */
function identityDisplayName(identity: QualifiedIdentityDto): string {
  return identity.alias || displayId(identity.id);
}

// ─── Component ─────────────────────────────────────────────────────

export function VoteCastingDialog({
  open,
  onOpenChange,
  selectedVotes,
  votingIdentities,
  onApplyVotes,
  onGoToActiveContests,
  onGoToScheduledVotes,
  votingInProgress = false,
}: VoteCastingDialogProps) {
  // Per-identity vote options
  const [identityOptions, setIdentityOptions] = useState<
    IdentityVoteOption[]
  >(() => buildDefaultOptions(votingIdentities));

  // "Set All" controls
  const [setAllMethod, setSetAllMethod] = useState<VoteMethod>("noVote");
  const [setAllDays, setSetAllDays] = useState(0);
  const [setAllHours, setSetAllHours] = useState(0);
  const [setAllMinutes, setSetAllMinutes] = useState(0);

  // Vote handling status
  const [status, setStatus] = useState<VoteHandlingStatus>({
    type: "notStarted",
  });

  // Reset state when dialog opens with new data
  const resetState = useCallback(() => {
    setIdentityOptions(buildDefaultOptions(votingIdentities));
    setSetAllMethod("noVote");
    setSetAllDays(0);
    setSetAllHours(0);
    setSetAllMinutes(0);
    setStatus({ type: "notStarted" });
  }, [votingIdentities]);

  // Validation
  const validationError = useMemo((): string | null => {
    if (selectedVotes.length === 0) return "No votes selected.";
    if (votingIdentities.length === 0) return "No voting identities loaded.";
    const hasAction = identityOptions.some((o) => o.method !== "noVote");
    if (!hasAction) return "Select at least one identity to cast or schedule votes.";
    // Check schedule times are valid (> 0 total)
    for (const opt of identityOptions) {
      if (opt.method === "schedule") {
        const totalMinutes =
          opt.scheduleDays * 1440 + opt.scheduleHours * 60 + opt.scheduleMinutes;
        if (totalMinutes <= 0) {
          return `Schedule time for ${identityDisplayName(votingIdentities.find((i) => i.id === opt.identityId) || { id: opt.identityId, alias: null } as QualifiedIdentityDto)} must be > 0.`;
        }
      }
    }
    return null;
  }, [selectedVotes, votingIdentities, identityOptions]);

  // Handle "Set All" apply
  const handleSetAll = useCallback(() => {
    setIdentityOptions((prev) =>
      prev.map((opt) => ({
        ...opt,
        method: setAllMethod,
        scheduleDays: setAllDays,
        scheduleHours: setAllHours,
        scheduleMinutes: setAllMinutes,
      })),
    );
  }, [setAllMethod, setAllDays, setAllHours, setAllMinutes]);

  // Update a single identity's vote option
  const updateIdentityOption = useCallback(
    (identityId: string, update: Partial<IdentityVoteOption>) => {
      setIdentityOptions((prev) =>
        prev.map((opt) =>
          opt.identityId === identityId ? { ...opt, ...update } : opt,
        ),
      );
    },
    [],
  );

  // Apply votes
  const handleApply = useCallback(async () => {
    const immediateIds = identityOptions
      .filter((o) => o.method === "castNow")
      .map((o) => o.identityId);
    const scheduledEntries = identityOptions
      .filter((o) => o.method === "schedule")
      .map((o) => ({
        voterId: o.identityId,
        unixTimestamp:
          Date.now() +
          o.scheduleDays * 86_400_000 +
          o.scheduleHours * 3_600_000 +
          o.scheduleMinutes * 60_000,
      }));

    if (immediateIds.length === 0 && scheduledEntries.length === 0) return;

    if (immediateIds.length > 0 && scheduledEntries.length > 0) {
      setStatus({ type: "casting", startTime: Date.now() });
    } else if (immediateIds.length > 0) {
      setStatus({ type: "casting", startTime: Date.now() });
    } else {
      setStatus({ type: "scheduling" });
    }

    try {
      await onApplyVotes(immediateIds, scheduledEntries);
      const results: VoteResultSummary = {
        totalCast: immediateIds.length,
        totalScheduled: scheduledEntries.length,
        castSucceeded: immediateIds.length,
        castFailed: 0,
        scheduleSucceeded: scheduledEntries.length,
        scheduleFailed: 0,
        errors: [],
      };
      setStatus({ type: "completed", results });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setStatus({ type: "failed", message });
    }
  }, [identityOptions, onApplyVotes]);

  // Handle dialog close
  const handleOpenChange = useCallback(
    (value: boolean) => {
      if (!value) {
        // Don't close while voting is in progress
        if (
          status.type === "casting" ||
          status.type === "scheduling" ||
          votingInProgress
        ) {
          return;
        }
        resetState();
      }
      onOpenChange(value);
    },
    [status, votingInProgress, resetState, onOpenChange],
  );

  // Determine if we're in a processing state
  const isProcessing =
    status.type === "casting" ||
    status.type === "scheduling" ||
    votingInProgress;

  // ─── Render ────────────────────────────────────────────────────────

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className="sm:max-w-2xl max-h-[85vh] overflow-y-auto"
        showCloseButton={!isProcessing}
        aria-label="Vote casting dialog"
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Vote className="h-5 w-5" />
            Voting
          </DialogTitle>
          <DialogDescription>
            {status.type === "completed"
              ? "Vote operation complete."
              : `Cast or schedule ${selectedVotes.length} vote${selectedVotes.length !== 1 ? "s" : ""} using your voting identities.`}
          </DialogDescription>
        </DialogHeader>

        {status.type === "notStarted" && (
          <VoteFormView
            selectedVotes={selectedVotes}
            votingIdentities={votingIdentities}
            identityOptions={identityOptions}
            setAllMethod={setAllMethod}
            setAllDays={setAllDays}
            setAllHours={setAllHours}
            setAllMinutes={setAllMinutes}
            validationError={validationError}
            onSetAllMethodChange={setSetAllMethod}
            onSetAllDaysChange={setSetAllDays}
            onSetAllHoursChange={setSetAllHours}
            onSetAllMinutesChange={setSetAllMinutes}
            onSetAll={handleSetAll}
            onUpdateIdentityOption={updateIdentityOption}
            onApply={handleApply}
            onCancel={() => handleOpenChange(false)}
          />
        )}

        {(status.type === "casting" || status.type === "scheduling") && (
          <VoteProgressView status={status} />
        )}

        {status.type === "completed" && (
          <VoteCompletedView
            results={status.results}
            onGoToActiveContests={() => {
              resetState();
              onOpenChange(false);
              onGoToActiveContests?.();
            }}
            onGoToScheduledVotes={() => {
              resetState();
              onOpenChange(false);
              onGoToScheduledVotes?.();
            }}
          />
        )}

        {status.type === "failed" && (
          <VoteFailedView
            message={status.message}
            onRetry={() => setStatus({ type: "notStarted" })}
            onClose={() => handleOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

// ─── Sub-views ─────────────────────────────────────────────────────

function buildDefaultOptions(
  identities: QualifiedIdentityDto[],
): IdentityVoteOption[] {
  return identities.map((identity) => ({
    identityId: identity.id,
    method: "noVote" as VoteMethod,
    scheduleDays: 0,
    scheduleHours: 0,
    scheduleMinutes: 0,
  }));
}

// ─── Form View ─────────────────────────────────────────────────────

interface VoteFormViewProps {
  selectedVotes: SelectedVote[];
  votingIdentities: QualifiedIdentityDto[];
  identityOptions: IdentityVoteOption[];
  setAllMethod: VoteMethod;
  setAllDays: number;
  setAllHours: number;
  setAllMinutes: number;
  validationError: string | null;
  onSetAllMethodChange: (method: VoteMethod) => void;
  onSetAllDaysChange: (days: number) => void;
  onSetAllHoursChange: (hours: number) => void;
  onSetAllMinutesChange: (minutes: number) => void;
  onSetAll: () => void;
  onUpdateIdentityOption: (
    identityId: string,
    update: Partial<IdentityVoteOption>,
  ) => void;
  onApply: () => void;
  onCancel: () => void;
}

function VoteFormView({
  selectedVotes,
  votingIdentities,
  identityOptions,
  setAllMethod,
  setAllDays,
  setAllHours,
  setAllMinutes,
  validationError,
  onSetAllMethodChange,
  onSetAllDaysChange,
  onSetAllHoursChange,
  onSetAllMinutesChange,
  onSetAll,
  onUpdateIdentityOption,
  onApply,
  onCancel,
}: VoteFormViewProps) {
  const hasScheduled = identityOptions.some((o) => o.method === "schedule");

  return (
    <div className="space-y-4">
      {/* Selected Votes Summary */}
      <div>
        <Label className="text-sm font-medium">
          Selected Votes ({selectedVotes.length})
        </Label>
        <div className="mt-1.5 max-h-32 overflow-y-auto rounded-md border p-2 space-y-1">
          {selectedVotes.map((vote) => (
            <div
              key={vote.contestedName}
              className="flex items-center justify-between text-sm"
            >
              <span className="font-mono font-medium">
                {vote.contestedName}
              </span>
              <div className="flex items-center gap-2">
                <Badge variant="secondary" className="text-xs">
                  {formatVoteChoice(vote.choice)}
                </Badge>
                {vote.endTime && (
                  <span className="text-xs text-muted-foreground">
                    ends {formatRelativeTime(vote.endTime)}
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>

      <Separator />

      {/* Set All Controls */}
      <div>
        <Label className="text-sm font-medium">Set All Identities</Label>
        <div className="mt-1.5 flex items-end gap-2">
          <div className="flex-1">
            <Select
              value={setAllMethod}
              onValueChange={(v) => onSetAllMethodChange(v as VoteMethod)}
            >
              <SelectTrigger size="sm" aria-label="Set all vote method">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="noVote">No Vote</SelectItem>
                <SelectItem value="castNow">Cast Now</SelectItem>
                <SelectItem value="schedule">Schedule</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {setAllMethod === "schedule" && (
            <ScheduleTimePicker
              days={setAllDays}
              hours={setAllHours}
              minutes={setAllMinutes}
              onDaysChange={onSetAllDaysChange}
              onHoursChange={onSetAllHoursChange}
              onMinutesChange={onSetAllMinutesChange}
            />
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={onSetAll}
            aria-label="Apply to all identities"
          >
            Apply to All
          </Button>
        </div>
      </div>

      <Separator />

      {/* Per-Identity Vote Options */}
      <div>
        <Label className="text-sm font-medium">
          Voting Identities ({votingIdentities.length})
        </Label>
        <div className="mt-1.5 space-y-2 max-h-48 overflow-y-auto">
          {votingIdentities.map((identity) => {
            const option = identityOptions.find(
              (o) => o.identityId === identity.id,
            );
            if (!option) return null;
            return (
              <IdentityVoteRow
                key={identity.id}
                identity={identity}
                option={option}
                onUpdate={(update) =>
                  onUpdateIdentityOption(identity.id, update)
                }
              />
            );
          })}
        </div>
      </div>

      {/* Schedule Warning */}
      {hasScheduled && (
        <div className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-400">
          <Clock className="mt-0.5 h-4 w-4 shrink-0" />
          <span>
            Dash Evo Tool must remain running and connected for scheduled votes
            to execute on time.
          </span>
        </div>
      )}

      {/* Validation Error */}
      {validationError && (
        <div
          className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
          role="alert"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>{validationError}</span>
        </div>
      )}

      {/* Actions */}
      <DialogFooter>
        <Button variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          onClick={onApply}
          disabled={validationError !== null}
          aria-label="Apply votes"
        >
          Apply Votes
        </Button>
      </DialogFooter>
    </div>
  );
}

// ─── Identity Vote Row ─────────────────────────────────────────────

interface IdentityVoteRowProps {
  identity: QualifiedIdentityDto;
  option: IdentityVoteOption;
  onUpdate: (update: Partial<IdentityVoteOption>) => void;
}

function IdentityVoteRow({
  identity,
  option,
  onUpdate,
}: IdentityVoteRowProps) {
  return (
    <div className="flex items-center gap-2 rounded-md border p-2">
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium truncate">
          {identityDisplayName(identity)}
        </div>
        <div className="text-xs text-muted-foreground">
          {identity.identityType}
        </div>
      </div>
      <Select
        value={option.method}
        onValueChange={(v) => onUpdate({ method: v as VoteMethod })}
      >
        <SelectTrigger
          size="sm"
          className="w-[120px]"
          aria-label={`Vote method for ${identityDisplayName(identity)}`}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="noVote">No Vote</SelectItem>
          <SelectItem value="castNow">Cast Now</SelectItem>
          <SelectItem value="schedule">Schedule</SelectItem>
        </SelectContent>
      </Select>
      {option.method === "schedule" && (
        <ScheduleTimePicker
          days={option.scheduleDays}
          hours={option.scheduleHours}
          minutes={option.scheduleMinutes}
          onDaysChange={(d) => onUpdate({ scheduleDays: d })}
          onHoursChange={(h) => onUpdate({ scheduleHours: h })}
          onMinutesChange={(m) => onUpdate({ scheduleMinutes: m })}
        />
      )}
    </div>
  );
}

// ─── Schedule Time Picker ──────────────────────────────────────────

interface ScheduleTimePickerProps {
  days: number;
  hours: number;
  minutes: number;
  onDaysChange: (days: number) => void;
  onHoursChange: (hours: number) => void;
  onMinutesChange: (minutes: number) => void;
}

function ScheduleTimePicker({
  days,
  hours,
  minutes,
  onDaysChange,
  onHoursChange,
  onMinutesChange,
}: ScheduleTimePickerProps) {
  return (
    <div className="flex items-center gap-1">
      <TimeInput
        value={days}
        onChange={onDaysChange}
        max={14}
        label="Days"
        suffix="d"
      />
      <TimeInput
        value={hours}
        onChange={onHoursChange}
        max={23}
        label="Hours"
        suffix="h"
      />
      <TimeInput
        value={minutes}
        onChange={onMinutesChange}
        max={59}
        label="Minutes"
        suffix="m"
      />
    </div>
  );
}

interface TimeInputProps {
  value: number;
  onChange: (value: number) => void;
  max: number;
  label: string;
  suffix: string;
}

function TimeInput({ value, onChange, max, label, suffix }: TimeInputProps) {
  return (
    <div className="flex items-center">
      <Input
        type="number"
        min={0}
        max={max}
        value={value}
        onChange={(e) => {
          const num = parseInt(e.target.value, 10);
          if (!isNaN(num) && num >= 0 && num <= max) {
            onChange(num);
          }
        }}
        className="w-14 h-8 text-center text-sm px-1"
        aria-label={label}
      />
      <span className="text-xs text-muted-foreground ml-0.5">{suffix}</span>
    </div>
  );
}

// ─── Progress View ─────────────────────────────────────────────────

interface VoteProgressViewProps {
  status: VoteHandlingStatus;
}

function VoteProgressView({ status }: VoteProgressViewProps) {
  return (
    <div className="flex flex-col items-center justify-center py-8 space-y-4">
      <Loader2 className="h-8 w-8 animate-spin text-primary" />
      <div className="text-center space-y-1">
        {status.type === "casting" && (
          <>
            <p className="text-sm font-medium">Casting votes...</p>
            <p className="text-xs text-muted-foreground" data-testid="elapsed-time">
              Time elapsed: {formatElapsed(status.startTime)}
            </p>
          </>
        )}
        {status.type === "scheduling" && (
          <p className="text-sm font-medium">Scheduling votes...</p>
        )}
      </div>
    </div>
  );
}

// ─── Completed View ────────────────────────────────────────────────

interface VoteCompletedViewProps {
  results: VoteResultSummary;
  onGoToActiveContests: () => void;
  onGoToScheduledVotes: () => void;
}

function VoteCompletedView({
  results,
  onGoToActiveContests,
  onGoToScheduledVotes,
}: VoteCompletedViewProps) {
  const allSucceeded =
    results.castFailed === 0 && results.scheduleFailed === 0;
  const allFailed =
    results.castSucceeded === 0 && results.scheduleSucceeded === 0;

  return (
    <div className="space-y-4 py-2">
      {/* Result Icon + Heading */}
      <div className="flex flex-col items-center text-center space-y-2">
        {allSucceeded ? (
          <>
            <div className="h-12 w-12 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center">
              <Check className="h-6 w-6 text-green-600 dark:text-green-400" />
            </div>
            <p className="text-sm font-medium" role="status">
              Successfully casted and scheduled all votes
            </p>
          </>
        ) : allFailed ? (
          <>
            <div className="h-12 w-12 rounded-full bg-destructive/10 flex items-center justify-center">
              <AlertCircle className="h-6 w-6 text-destructive" />
            </div>
            <p className="text-sm font-medium text-destructive" role="alert">
              No votes succeeded
            </p>
          </>
        ) : (
          <>
            <div className="h-12 w-12 rounded-full bg-amber-100 dark:bg-amber-900/30 flex items-center justify-center">
              <AlertCircle className="h-6 w-6 text-amber-600 dark:text-amber-400" />
            </div>
            <p className="text-sm font-medium text-amber-700 dark:text-amber-400" role="status">
              Only some votes succeeded
            </p>
          </>
        )}
      </div>

      {/* Result Details */}
      <div className="rounded-md border p-3 space-y-1 text-sm">
        {results.totalCast > 0 && (
          <div className="flex justify-between">
            <span className="text-muted-foreground">Votes cast:</span>
            <span>
              {results.castSucceeded}/{results.totalCast} succeeded
            </span>
          </div>
        )}
        {results.totalScheduled > 0 && (
          <div className="flex justify-between">
            <span className="text-muted-foreground">Votes scheduled:</span>
            <span>
              {results.scheduleSucceeded}/{results.totalScheduled} succeeded
            </span>
          </div>
        )}
      </div>

      {/* Errors */}
      {results.errors.length > 0 && (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 space-y-1">
          <p className="text-xs font-medium text-destructive">Errors:</p>
          {results.errors.map((err, i) => (
            <p key={i} className="text-xs text-destructive">
              {err}
            </p>
          ))}
        </div>
      )}

      {/* Navigation Buttons */}
      <DialogFooter>
        <Button variant="outline" onClick={onGoToActiveContests}>
          Go to Active Contests
        </Button>
        {results.totalScheduled > 0 && (
          <Button onClick={onGoToScheduledVotes}>
            Go to Scheduled Votes
          </Button>
        )}
      </DialogFooter>
    </div>
  );
}

// ─── Failed View ───────────────────────────────────────────────────

interface VoteFailedViewProps {
  message: string;
  onRetry: () => void;
  onClose: () => void;
}

function VoteFailedView({ message, onRetry, onClose }: VoteFailedViewProps) {
  return (
    <div className="space-y-4 py-2">
      <div className="flex flex-col items-center text-center space-y-2">
        <div className="h-12 w-12 rounded-full bg-destructive/10 flex items-center justify-center">
          <AlertCircle className="h-6 w-6 text-destructive" />
        </div>
        <p className="text-sm font-medium text-destructive" role="alert">
          Error casting and scheduling votes
        </p>
      </div>
      <div
        className={cn(
          "rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive",
        )}
      >
        {message}
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          Close
        </Button>
        <Button onClick={onRetry}>Try Again</Button>
      </DialogFooter>
    </div>
  );
}
