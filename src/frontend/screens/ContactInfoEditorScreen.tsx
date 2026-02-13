import { useState, useEffect, useMemo, useCallback } from "react";
import {
  ArrowLeft,
  User,
  Info,
  Loader2,
  Save,
  X,
  Lock,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback/LoadingSpinner";
import { WalletUnlockDialog } from "@/components/shared";
import type { WalletUnlockResult } from "@/components/shared/WalletUnlockDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { toastError } from "@/lib/toastError";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { useTaskListener } from "@/hooks/useTaskListener";
import type { TaskResultEvent, TaskErrorEvent } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

const PRIVATE_INFO_HELP = [
  "This information is encrypted and stored on Platform.",
  "It is NEVER shared with the contact — only you can decrypt it.",
  "Only you can see these nicknames and notes.",
  "Hidden contacts can still send you payments.",
  "Use this to organize and remember your contacts.",
];

// ─── ContactInfoEditorScreen ──────────────────────────────────────────

interface ContactInfoEditorScreenProps {
  contactId: string;
}

export function ContactInfoEditorScreen({ contactId }: ContactInfoEditorScreenProps) {
  const navigate = useNavigate();
  const {
    selectedIdentityId,
    contacts,
    contactsError,
    loadContactPrivateInfo,
    saveContactPrivateInfo,
    updateContactInfo,
  } = useDashPayStore();
  const identities = useIdentityStore((s) => s.identities);
  const hdWallets = useWalletStore((s) => s.hdWallets);
  const unlockWallet = useWalletStore((s) => s.unlockWallet);

  // ── Associated wallet ──
  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  const associatedWallet = useMemo(() => {
    if (!selectedIdentity) return null;
    const hashes = selectedIdentity.associatedWalletHashes;
    if (!hashes || hashes.length === 0) return null;
    return hdWallets.find((w) => hashes.includes(w.seedHash)) ?? null;
  }, [selectedIdentity, hdWallets]);

  const walletAlias = associatedWallet?.alias ?? "Wallet";

  // ── State ──
  const [loading, setLoading] = useState(true);
  const [nickname, setNickname] = useState("");
  const [note, setNote] = useState("");
  const [isHidden, setIsHidden] = useState(false);
  const [accountInput, setAccountInput] = useState("");
  const [acceptedAccounts, setAcceptedAccounts] = useState<number[]>([]);
  const [saving, setSaving] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [message, setMessage] = useState<{ text: string; type: "success" | "error" | "info" } | null>(null);
  const [showInfoPopup, setShowInfoPopup] = useState(false);
  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(null);
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<Set<string>>(new Set());

  // ── Derived: wallet locked ──
  const walletLocked =
    !!associatedWallet &&
    associatedWallet.usesPassword &&
    !walletUnlockedHashes.has(associatedWallet.seedHash);

  // ── Derived data ──
  const contact = useMemo(
    () => contacts.find((c) => c.contactIdentityId === contactId) ?? null,
    [contacts, contactId],
  );

  const contactDisplayName = useMemo(() => {
    if (contact?.displayName?.trim()) return contact.displayName.trim();
    if (contact?.username?.trim()) return `@${contact.username.trim()}`;
    return contactId;
  }, [contact, contactId]);

  // ── Load existing data on mount ──
  useEffect(() => {
    let cancelled = false;

    async function loadData() {
      setLoading(true);
      try {
        const info = await loadContactPrivateInfo(contactId);
        if (!cancelled && info) {
          setNickname(info.nickname ?? "");
          setNote(info.notes ?? "");
          setIsHidden(info.isHidden ?? false);
        }
      } catch {
        // Private info may not exist yet
      }
      if (!cancelled) setLoading(false);
    }

    loadData();

    return () => {
      cancelled = true;
    };
  }, [contactId, loadContactPrivateInfo]);

  // ── Handlers ──
  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contact-details/$contactId", params: { contactId } });
  }, [navigate, contactId]);

  const handleParseAccounts = useCallback(() => {
    const parsed: number[] = [];
    for (const part of accountInput.split(",")) {
      const trimmed = part.trim();
      if (trimmed === "") continue;
      const index = parseInt(trimmed, 10);
      if (!isNaN(index) && index >= 0 && !parsed.includes(index)) {
        parsed.push(index);
      }
    }
    parsed.sort((a, b) => a - b);
    setAcceptedAccounts(parsed);
    setAccountInput(parsed.join(", "));
  }, [accountInput]);

  const handleSave = useCallback(async () => {
    if (!selectedIdentityId) return;
    if (walletLocked) {
      setShowWalletUnlock(true);
      return;
    }
    setSaving(true);
    setMessage(null);

    try {
      // Save to local DB
      await saveContactPrivateInfo({
        contactId,
        nickname,
        notes: note,
        isHidden,
      });

      // Dispatch Platform update — returns taskId or null on IPC failure
      const id = await updateContactInfo({
        contactId,
        nickname: nickname || null,
        note: note || null,
        isHidden,
        acceptedAccounts,
      });

      if (!id) {
        // IPC dispatch failed — store already has contactsError
        const storeError = useDashPayStore.getState().contactsError;
        setMessage({ text: storeError ?? "Failed to update contact info.", type: "error" });
        setSaving(false);
        return;
      }

      // Task dispatched — stay in saving state, wait for task event
      setTaskId(id);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setMessage({ text: msg, type: "error" });
      toastError(msg);
      setSaving(false);
    }
  }, [
    selectedIdentityId,
    walletLocked,
    contactId,
    nickname,
    note,
    isHidden,
    acceptedAccounts,
    saveContactPrivateInfo,
    updateContactInfo,
  ]);

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
        handleSave();
      }
    },
    [associatedWallet, unlockWallet, handleSave],
  );

  const handleCancel = useCallback(() => {
    navigate({ to: "/dashpay/contact-details/$contactId", params: { contactId } });
  }, [navigate, contactId]);

  // Listen for task completion/error
  useTaskListener(
    taskId,
    useCallback((_event: TaskResultEvent) => {
      setMessage({ text: "Contact information updated successfully", type: "success" });
      setSaving(false);
      setTaskId(null);
    }, []),
    useCallback((event: TaskErrorEvent) => {
      setMessage({ text: event.message, type: "error" });
      setSaving(false);
      setTaskId(null);
    }, []),
  );

  // ── Render ──
  if (!selectedIdentityId) {
    return (
      <Island>
        <EmptyState
          icon={User}
          title="No Identity Selected"
          description="Select an identity from the sidebar to edit contact details."
        />
      </Island>
    );
  }

  return (
    <Island>
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <Button variant="ghost" size="sm" onClick={handleBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <h2 className="text-lg font-semibold flex-1">Edit Private Contact Details</h2>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={() => setShowInfoPopup(true)}
              aria-label="About private contact information"
            >
              <Info className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="max-w-xs">
            <ul className="space-y-1 text-xs">
              {PRIVATE_INFO_HELP.map((line, i) => (
                <li key={i}>• {line}</li>
              ))}
            </ul>
          </TooltipContent>
        </Tooltip>
      </div>

      {/* Info Dialog */}
      <Dialog open={showInfoPopup} onOpenChange={setShowInfoPopup}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>About Private Contact Information</DialogTitle>
            <DialogDescription asChild>
              <ul className="space-y-2 mt-2">
                {PRIVATE_INFO_HELP.map((line, i) => (
                  <li key={i} className="text-sm">• {line}</li>
                ))}
              </ul>
            </DialogDescription>
          </DialogHeader>
        </DialogContent>
      </Dialog>

      {/* Message */}
      {message && (
        <div
          className={cn(
            "mb-4 rounded-md px-4 py-2 text-sm",
            message.type === "success" &&
              "bg-green-50 text-green-700 dark:bg-green-950 dark:text-green-300",
            message.type === "error" &&
              "bg-destructive/10 text-destructive",
            message.type === "info" &&
              "bg-blue-50 text-blue-700 dark:bg-blue-950 dark:text-blue-300",
          )}
          role="status"
        >
          {message.text}
        </div>
      )}

      {/* Error from store */}
      {contactsError && (
        <div className="mb-4 rounded-md bg-destructive/10 px-4 py-2 text-sm text-destructive" role="alert">
          {contactsError}
        </div>
      )}

      {/* Loading */}
      {loading ? (
        <LoadingSpinner label="Loading contact info..." />
      ) : (
        <div className="space-y-6">
          {/* Contact identifier */}
          <div className="rounded-lg border bg-muted/50 px-4 py-3">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">Contact:</span>
              <span className="text-sm text-muted-foreground">{contactDisplayName}</span>
            </div>
            <p className="text-xs text-muted-foreground font-mono mt-1">{contactId}</p>
          </div>

          <Separator />

          {/* Nickname field */}
          <div className="space-y-2">
            <Label htmlFor="contact-nickname">Private Nickname</Label>
            <p className="text-xs text-muted-foreground">
              Give this contact a custom name that ONLY YOU will see
            </p>
            <Input
              id="contact-nickname"
              value={nickname}
              onChange={(e) => setNickname(e.target.value)}
              placeholder="e.g., 'Mom', 'Boss', 'Alice from work'"
              disabled={saving}
            />
          </div>

          {/* Note field */}
          <div className="space-y-2">
            <Label htmlFor="contact-note">Private Note</Label>
            <p className="text-xs text-muted-foreground">
              Add notes about this contact (only visible to you)
            </p>
            <Textarea
              id="contact-note"
              value={note}
              onChange={(e) => setNote(e.target.value)}
              placeholder="e.g., 'Met at Dash conference 2024', 'Owes me for lunch'"
              rows={5}
              disabled={saving}
            />
          </div>

          {/* Hidden checkbox */}
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Checkbox
                id="contact-hidden"
                checked={isHidden}
                onCheckedChange={(checked) => setIsHidden(checked === true)}
                disabled={saving}
              />
              <Label htmlFor="contact-hidden" className="text-sm cursor-pointer">
                Hide this contact from my list
              </Label>
            </div>
            {isHidden ? (
              <p className="text-xs text-amber-600 dark:text-amber-400">
                Hidden contacts won't appear in your contact list but can still send you payments
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">
                Contact will appear in your contact list
              </p>
            )}
          </div>

          <Separator />

          {/* Accepted Account Indices */}
          <div className="space-y-2">
            <Label htmlFor="account-indices">Accepted Account Indices</Label>
            <p className="text-xs text-muted-foreground">
              Specify which account indices this contact can pay to (comma-separated)
            </p>
            <div className="flex items-center gap-2">
              <Input
                id="account-indices"
                value={accountInput}
                onChange={(e) => setAccountInput(e.target.value)}
                placeholder="e.g., 0, 1, 2"
                className="max-w-[250px]"
                disabled={saving}
              />
              <Button
                variant="outline"
                size="sm"
                onClick={handleParseAccounts}
                disabled={saving}
              >
                Parse
              </Button>
            </div>
            {acceptedAccounts.length > 0 ? (
              <p className="text-xs text-muted-foreground">
                Accepted accounts: [{acceptedAccounts.join(", ")}]
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">
                All accounts accepted (default)
              </p>
            )}
          </div>

          <Separator />

          {/* Wallet locked warning */}
          {walletLocked && (
            <div className="flex items-center gap-2">
              <Lock className="h-3.5 w-3.5 text-amber-500" />
              <span className="text-xs text-amber-600 dark:text-amber-400">
                Wallet is locked.
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

          {/* Action buttons */}
          <div className="flex items-center gap-3">
            {saving ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                <span className="text-sm text-muted-foreground">Saving...</span>
              </>
            ) : (
              <>
                <Button onClick={handleSave} disabled={saving || walletLocked}>
                  <Save className="h-4 w-4 mr-1" />
                  Save Changes
                </Button>
                <Button variant="outline" onClick={handleCancel} disabled={saving}>
                  <X className="h-4 w-4 mr-1" />
                  Cancel
                </Button>
              </>
            )}
          </div>
        </div>
      )}

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
    </Island>
  );
}
