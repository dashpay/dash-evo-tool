import { useState, useMemo, useCallback } from "react";
import {
  User,
  Inbox,
  Send,
  Loader2,
  Check,
  X,
  AlertTriangle,
  Lock,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback/LoadingSpinner";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import { WalletUnlockDialog } from "@/components/shared";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import type { StoredContactRequestDto } from "@/bindings";
import { displayId } from "@/lib/utils";

// ─── Helpers ─────────────────────────────────────────────────────────

function getRequestDisplayName(
  request: StoredContactRequestDto,
  direction: "incoming" | "outgoing",
): string {
  if (direction === "incoming") {
    return request.fromUsername || displayId(request.fromIdentityId);
  }
  return request.toUsername || displayId(request.toIdentityId);
}

function getRequestIdentityId(
  request: StoredContactRequestDto,
  direction: "incoming" | "outgoing",
): string {
  return direction === "incoming"
    ? request.fromIdentityId
    : request.toIdentityId;
}

function formatTimestamp(timestamp: number): string {
  const now = Date.now();
  const diff = now - timestamp * 1000;
  const seconds = Math.floor(diff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (days > 0) return `${days}d ago`;
  if (hours > 0) return `${hours}h ago`;
  if (minutes > 0) return `${minutes}m ago`;
  return "just now";
}

// ─── IncomingRequestCard ─────────────────────────────────────────────

interface IncomingRequestCardProps {
  request: StoredContactRequestDto;
  isAccepting: boolean;
  isRejecting: boolean;
  onAccept: (request: StoredContactRequestDto) => void;
  onReject: (request: StoredContactRequestDto) => void;
}

function IncomingRequestCard({
  request,
  isAccepting,
  isRejecting,
  onAccept,
  onReject,
}: IncomingRequestCardProps) {
  const displayName = getRequestDisplayName(request, "incoming");
  const identityId = getRequestIdentityId(request, "incoming");
  const isProcessing = isAccepting || isRejecting;
  const isAccepted = request.status === "accepted";
  const isRejected = request.status === "rejected";

  return (
    <div
      className="flex items-center gap-4 rounded-lg border bg-card p-4 transition-colors hover:bg-muted/30"
      data-testid="incoming-request-card"
    >
      {/* Avatar */}
      <div className="shrink-0 h-10 w-10 rounded-full bg-muted flex items-center justify-center">
        <User className="h-5 w-5 text-muted-foreground" />
      </div>

      {/* Request info */}
      <div className="flex-1 min-w-0 space-y-0.5">
        <p className="text-sm font-medium truncate">{displayName}</p>
        <p className="text-xs text-muted-foreground font-mono truncate">
          ID: {identityId}
        </p>
        {request.accountLabel && (
          <p className="text-xs text-muted-foreground">
            Account: {request.accountLabel}
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          Received: {formatTimestamp(request.createdAt)}
        </p>
      </div>

      {/* Actions */}
      <div className="flex items-center gap-2 shrink-0">
        {isAccepted ? (
          <Badge
            variant="outline"
            className="text-xs text-green-600 border-green-600"
          >
            <Check className="mr-1 h-3 w-3" />
            Accepted
          </Badge>
        ) : isRejected ? (
          <Badge
            variant="outline"
            className="text-xs text-destructive border-destructive"
          >
            <X className="mr-1 h-3 w-3" />
            Rejected
          </Badge>
        ) : (
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() => onReject(request)}
              disabled={isProcessing}
            >
              {isRejecting ? (
                <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
              ) : (
                <X className="mr-1 h-3.5 w-3.5" />
              )}
              Reject
            </Button>
            <Button
              size="sm"
              onClick={() => onAccept(request)}
              disabled={isProcessing}
            >
              {isAccepting ? (
                <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
              ) : (
                <Check className="mr-1 h-3.5 w-3.5" />
              )}
              Accept
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

// ─── OutgoingRequestCard ─────────────────────────────────────────────

interface OutgoingRequestCardProps {
  request: StoredContactRequestDto;
}

function OutgoingRequestCard({ request }: OutgoingRequestCardProps) {
  const displayName = getRequestDisplayName(request, "outgoing");
  const identityId = getRequestIdentityId(request, "outgoing");

  return (
    <div
      className="flex items-center gap-4 rounded-lg border bg-card p-4 transition-colors hover:bg-muted/30"
      data-testid="outgoing-request-card"
    >
      {/* Avatar */}
      <div className="shrink-0 h-10 w-10 rounded-full bg-muted flex items-center justify-center">
        <User className="h-5 w-5 text-muted-foreground" />
      </div>

      {/* Request info */}
      <div className="flex-1 min-w-0 space-y-0.5">
        <p className="text-sm font-medium truncate">To: {displayName}</p>
        {request.toUsername && (
          <p className="text-xs text-muted-foreground truncate">
            @{request.toUsername}
          </p>
        )}
        <p className="text-xs text-muted-foreground font-mono truncate">
          ID: {identityId}
        </p>
        {request.accountLabel && (
          <p className="text-xs text-muted-foreground">
            Account: {request.accountLabel}
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          Status: <span className="capitalize">{request.status}</span>
        </p>
        <p className="text-xs text-muted-foreground">
          Sent: {formatTimestamp(request.createdAt)}
        </p>
      </div>

      {/* Note */}
      <div className="shrink-0">
        <span className="text-xs text-muted-foreground italic">
          Cannot be cancelled once sent
        </span>
      </div>
    </div>
  );
}

// ─── ContactRequests ─────────────────────────────────────────────────

interface ContactRequestsProps {
  onAddContact?: () => void;
}

export function ContactRequests({ onAddContact }: ContactRequestsProps) {
  const incomingRequests = useDashPayStore((s) => s.incomingRequests);
  const outgoingRequests = useDashPayStore((s) => s.outgoingRequests);
  const requestsLoading = useDashPayStore((s) => s.requestsLoading);
  const requestsError = useDashPayStore((s) => s.requestsError);
  const acceptingIds = useDashPayStore((s) => s.acceptingIds);
  const rejectingIds = useDashPayStore((s) => s.rejectingIds);
  const acceptContactRequest = useDashPayStore(
    (s) => s.acceptContactRequest,
  );
  const rejectContactRequest = useDashPayStore(
    (s) => s.rejectContactRequest,
  );
  const selectedIdentityId = useDashPayStore((s) => s.selectedIdentityId);
  const identities = useIdentityStore((s) => s.identities);
  const wallets = useWalletStore((s) => s.hdWallets);
  const unlockWallet = useWalletStore((s) => s.unlockWallet);

  // ── Associated wallet & lock state ──
  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  const associatedWallet = useMemo(() => {
    if (!selectedIdentity) return null;
    const hashes = selectedIdentity.associatedWalletHashes;
    if (!hashes || hashes.length === 0) return null;
    return wallets.find((w) => hashes.includes(w.seedHash)) ?? null;
  }, [selectedIdentity, wallets]);

  const walletAlias = associatedWallet?.alias ?? "Wallet";

  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(null);
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<Set<string>>(new Set());
  const [pendingAction, setPendingAction] = useState<{
    type: "accept" | "reject";
    dbRowId: number;
    platformId: string;
  } | null>(null);

  const walletLocked =
    !!associatedWallet &&
    associatedWallet.usesPassword &&
    !walletUnlockedHashes.has(associatedWallet.seedHash);

  const [activeSubTab, setActiveSubTab] = useState<"incoming" | "outgoing">(
    "incoming",
  );
  const [confirmDialog, setConfirmDialog] = useState<{
    open: boolean;
    type: "accept" | "reject";
    request: StoredContactRequestDto | null;
  }>({ open: false, type: "accept", request: null });

  const handleAcceptClick = useCallback(
    (request: StoredContactRequestDto) => {
      setConfirmDialog({ open: true, type: "accept", request });
    },
    [],
  );

  const handleRejectClick = useCallback(
    (request: StoredContactRequestDto) => {
      setConfirmDialog({ open: true, type: "reject", request });
    },
    [],
  );

  const dispatchAction = useCallback(
    (type: "accept" | "reject", dbRowId: number, platformId: string) => {
      if (type === "accept") {
        acceptContactRequest(dbRowId, platformId);
      } else {
        rejectContactRequest(dbRowId, platformId);
      }
    },
    [acceptContactRequest, rejectContactRequest],
  );

  const handleConfirmResult = useCallback(
    (status: "confirmed" | "canceled") => {
      if (status === "confirmed" && confirmDialog.request) {
        const platformId = confirmDialog.request.platformDocumentId;
        if (!platformId) {
          // Old DB row without platform document ID — trigger a refresh
          useDashPayStore.getState().refreshContactRequests();
          setConfirmDialog({ open: false, type: "accept", request: null });
          return;
        }
        const dbRowId = confirmDialog.request.id;
        const actionType = confirmDialog.type;

        // Gate on wallet lock
        if (walletLocked) {
          setPendingAction({ type: actionType, dbRowId, platformId });
          setShowWalletUnlock(true);
          setConfirmDialog({ open: false, type: "accept", request: null });
          return;
        }

        dispatchAction(actionType, dbRowId, platformId);
      }
      setConfirmDialog({ open: false, type: "accept", request: null });
    },
    [confirmDialog, walletLocked, dispatchAction],
  );

  const handleWalletUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && associatedWallet) {
        setWalletUnlockError(null);
        const error = await unlockWallet(
          { type: "hd", seedHash: associatedWallet.seedHash },
          result.password,
        );
        if (error) {
          setWalletUnlockError(error);
          return;
        }
        setWalletUnlockedHashes(
          (prev) => new Set([...prev, associatedWallet.seedHash]),
        );
        setShowWalletUnlock(false);

        // Replay the pending action
        if (pendingAction) {
          dispatchAction(pendingAction.type, pendingAction.dbRowId, pendingAction.platformId);
          setPendingAction(null);
        }
      }
    },
    [associatedWallet, unlockWallet, pendingAction, dispatchAction],
  );

  if (requestsLoading) {
    return (
      <div className="flex-1 flex items-center justify-center py-12">
        <LoadingSpinner size="lg" label="Loading requests..." />
      </div>
    );
  }

  const confirmDisplayName = confirmDialog.request
    ? getRequestDisplayName(confirmDialog.request, "incoming")
    : "";

  return (
    <div className="flex flex-col flex-1 min-h-0 space-y-4">
      {/* Error banner */}
      {requestsError && (
        <div
          className="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive flex items-center gap-2"
          role="alert"
        >
          <AlertTriangle className="h-4 w-4 shrink-0" />
          <span>{requestsError}</span>
        </div>
      )}

      {/* Wallet locked warning */}
      {walletLocked && (
        <div className="flex items-center gap-2">
          <Lock className="h-3.5 w-3.5 text-amber-500" />
          <span className="text-xs text-amber-600 dark:text-amber-400">
            Wallet is locked. Unlock to accept or reject requests.
          </span>
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0 text-xs"
            onClick={() => setShowWalletUnlock(true)}
          >
            Unlock Wallet
          </Button>
        </div>
      )}

      {/* Sub-tabs */}
      <Tabs
        value={activeSubTab}
        onValueChange={(v) =>
          setActiveSubTab(v as "incoming" | "outgoing")
        }
      >
        <TabsList>
          <TabsTrigger value="incoming" className="gap-1.5">
            <Inbox className="h-3.5 w-3.5" />
            Incoming
            {incomingRequests.filter((r) => r.status === "pending").length >
              0 && (
              <Badge
                variant="secondary"
                className="ml-1 h-5 min-w-[20px] rounded-full px-1.5 text-xs"
              >
                {
                  incomingRequests.filter((r) => r.status === "pending")
                    .length
                }
              </Badge>
            )}
          </TabsTrigger>
          <TabsTrigger value="outgoing" className="gap-1.5">
            <Send className="h-3.5 w-3.5" />
            Outgoing
            {outgoingRequests.length > 0 && (
              <Badge
                variant="secondary"
                className="ml-1 h-5 min-w-[20px] rounded-full px-1.5 text-xs"
              >
                {outgoingRequests.length}
              </Badge>
            )}
          </TabsTrigger>
        </TabsList>
      </Tabs>

      {/* Tab content */}
      <div className="flex-1 overflow-y-auto">
        {activeSubTab === "incoming" ? (
          <IncomingRequestsList
            requests={incomingRequests}
            acceptingIds={acceptingIds}
            rejectingIds={rejectingIds}
            onAccept={handleAcceptClick}
            onReject={handleRejectClick}
          />
        ) : (
          <OutgoingRequestsList
            requests={outgoingRequests}
            onAddContact={onAddContact}
          />
        )}
      </div>

      {/* Confirmation dialogs */}
      <ConfirmationDialog
        open={confirmDialog.open && confirmDialog.type === "accept"}
        onOpenChange={(open) => {
          if (!open) handleConfirmResult("canceled");
        }}
        title="Accept Contact Request"
        message={`Are you sure you want to accept the contact request from ${confirmDisplayName}?`}
        confirmText="Accept"
        cancelText="Cancel"
        onResult={handleConfirmResult}
      />
      <ConfirmationDialog
        open={confirmDialog.open && confirmDialog.type === "reject"}
        onOpenChange={(open) => {
          if (!open) handleConfirmResult("canceled");
        }}
        title="Reject Contact Request"
        message={`Are you sure you want to reject the contact request from ${confirmDisplayName}?`}
        confirmText="Reject"
        cancelText="Cancel"
        danger
        onResult={handleConfirmResult}
      />

      {/* Wallet unlock dialog */}
      {associatedWallet && (
        <WalletUnlockDialog
          open={showWalletUnlock}
          onOpenChange={setShowWalletUnlock}
          walletAlias={walletAlias}
          error={walletUnlockError}
          passwordHint={associatedWallet.passwordHint ?? null}
          onResult={handleWalletUnlockResult}
        />
      )}
    </div>
  );
}

// ─── IncomingRequestsList ────────────────────────────────────────────

interface IncomingRequestsListProps {
  requests: StoredContactRequestDto[];
  acceptingIds: Set<number>;
  rejectingIds: Set<number>;
  onAccept: (request: StoredContactRequestDto) => void;
  onReject: (request: StoredContactRequestDto) => void;
}

function IncomingRequestsList({
  requests,
  acceptingIds,
  rejectingIds,
  onAccept,
  onReject,
}: IncomingRequestsListProps) {
  if (requests.length === 0) {
    return (
      <EmptyState
        icon={Inbox}
        title="No Incoming Requests"
        description="You don't have any pending contact requests."
      />
    );
  }

  return (
    <div className="space-y-2" role="list" aria-label="Incoming requests">
      {requests.map((req) => (
        <div role="listitem" key={req.id}>
          <IncomingRequestCard
            request={req}
            isAccepting={acceptingIds.has(req.id)}
            isRejecting={rejectingIds.has(req.id)}
            onAccept={onAccept}
            onReject={onReject}
          />
        </div>
      ))}
    </div>
  );
}

// ─── OutgoingRequestsList ────────────────────────────────────────────

interface OutgoingRequestsListProps {
  requests: StoredContactRequestDto[];
  onAddContact?: () => void;
}

function OutgoingRequestsList({
  requests,
  onAddContact,
}: OutgoingRequestsListProps) {
  if (requests.length === 0) {
    return (
      <EmptyState
        icon={Send}
        title="No Outgoing Requests"
        description="You haven't sent any contact requests."
        actionLabel="Add Contact"
        onAction={onAddContact}
      />
    );
  }

  return (
    <div className="space-y-2" role="list" aria-label="Outgoing requests">
      {requests.map((req) => (
        <div role="listitem" key={req.id}>
          <OutgoingRequestCard request={req} />
        </div>
      ))}
    </div>
  );
}
