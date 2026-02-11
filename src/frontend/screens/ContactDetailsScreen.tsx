import { useState, useEffect, useMemo, useCallback } from "react";
import {
  ArrowLeft,
  User,
  Pencil,
  Info,
  Loader2,
  Save,
  X,
  RefreshCw,
  ArrowUpRight,
  ArrowDownLeft,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback/LoadingSpinner";
import { CopyButton } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
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
import { useDashPayStore } from "@/stores/dashpayStore";
import { formatAmount } from "@/components/shared/AmountInput";
import type { StoredPaymentDto, ContactPrivateInfoDto } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

const PRIVATE_INFO_HELP = [
  "This information is encrypted and stored on Platform.",
  "It is never shared with the contact — only you can decrypt it.",
  "Only you can see these nicknames and notes.",
  "Use this to organize and remember your contacts.",
];

// ─── Helpers ──────────────────────────────────────────────────────────

function getContactDisplayName(
  contact: StoredContactDto | null,
  privateInfo: ContactPrivateInfoDto | null,
): string {
  if (privateInfo?.nickname?.trim()) return privateInfo.nickname.trim();
  if (contact?.displayName?.trim()) return contact.displayName.trim();
  if (contact?.username?.trim()) return contact.username.trim();
  return "Unknown";
}

function formatRelativeTime(timestamp: number): string {
  const now = Date.now();
  const diff = now - timestamp * 1000;
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  return `${months}mo ago`;
}

// ─── ContactDetailsScreen ─────────────────────────────────────────────

interface ContactDetailsScreenProps {
  contactId: string;
}

export function ContactDetailsScreen({ contactId }: ContactDetailsScreenProps) {
  const navigate = useNavigate();
  const {
    selectedIdentityId,
    contacts,
    payments,
    paymentsLoading,
    contactsError,
    fetchContactProfile,
    loadPayments,
    loadContactPrivateInfo,
    saveContactPrivateInfo,
    updateContactInfo,
  } = useDashPayStore();

  // ── State ──
  const [loading, setLoading] = useState(true);
  const [privateInfo, setPrivateInfo] = useState<ContactPrivateInfoDto | null>(null);
  const [editing, setEditing] = useState(false);
  const [editNickname, setEditNickname] = useState("");
  const [editNote, setEditNote] = useState("");
  const [editHidden, setEditHidden] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ text: string; type: "success" | "error" | "info" } | null>(null);
  const [showInfoPopup, setShowInfoPopup] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  // ── Derived data ──
  const contact = useMemo(
    () => contacts.find((c) => c.contactIdentityId === contactId) ?? null,
    [contacts, contactId],
  );

  const contactPayments = useMemo(() => {
    if (!selectedIdentityId) return [];
    return payments.filter(
      (p) =>
        p.fromIdentityId === contactId || p.toIdentityId === contactId,
    );
  }, [payments, contactId, selectedIdentityId]);

  const displayName = useMemo(
    () => getContactDisplayName(contact, privateInfo),
    [contact, privateInfo],
  );

  // ── Load data on mount ──
  useEffect(() => {
    let cancelled = false;

    async function loadData() {
      setLoading(true);
      try {
        const info = await loadContactPrivateInfo(contactId);
        if (!cancelled) {
          setPrivateInfo(info);
        }
      } catch {
        // Ignore — private info may not exist
      }
      if (!cancelled) setLoading(false);
    }

    loadData();
    loadPayments();

    // Fetch latest profile from platform
    fetchContactProfile(contactId);

    return () => {
      cancelled = true;
    };
  }, [contactId, loadContactPrivateInfo, loadPayments, fetchContactProfile]);

  // ── Handlers ──
  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contacts" });
  }, [navigate]);

  const handleStartEdit = useCallback(() => {
    setEditNickname(privateInfo?.nickname ?? "");
    setEditNote(privateInfo?.notes ?? "");
    setEditHidden(privateInfo?.isHidden ?? false);
    setEditing(true);
    setMessage(null);
  }, [privateInfo]);

  const handleCancelEdit = useCallback(() => {
    setEditing(false);
  }, []);

  const handleSave = useCallback(async () => {
    if (!selectedIdentityId) return;
    setSaving(true);
    setMessage(null);

    try {
      // Save to local DB
      await saveContactPrivateInfo({
        contactId,
        nickname: editNickname,
        notes: editNote,
        isHidden: editHidden,
      });

      // Update on Platform
      await updateContactInfo({
        contactId,
        nickname: editNickname || null,
        note: editNote || null,
        isHidden: editHidden,
        acceptedAccounts: [],
      });

      // Update local state
      setPrivateInfo({
        nickname: editNickname,
        notes: editNote,
        isHidden: editHidden,
      });

      setEditing(false);
      setMessage({ text: "Contact info saved to Platform", type: "success" });
    } catch (e) {
      setMessage({
        text: e instanceof Error ? e.message : String(e),
        type: "error",
      });
    } finally {
      setSaving(false);
    }
  }, [
    selectedIdentityId,
    contactId,
    editNickname,
    editNote,
    editHidden,
    saveContactPrivateInfo,
    updateContactInfo,
  ]);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    setMessage(null);
    try {
      await fetchContactProfile(contactId);
      setMessage({ text: "Profile refreshed", type: "info" });
    } catch {
      setMessage({ text: "Failed to refresh profile", type: "error" });
    } finally {
      setRefreshing(false);
    }
  }, [contactId, fetchContactProfile]);

  const handlePay = useCallback(() => {
    navigate({ to: "/dashpay/payments", search: { contactId } });
  }, [navigate, contactId]);

  // ── Render ──
  if (!selectedIdentityId) {
    return (
      <Island>
        <EmptyState
          icon={User}
          title="No Identity Selected"
          description="Select an identity from the sidebar to view contact details."
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
        <h2 className="text-lg font-semibold flex-1">Contact Details</h2>
        <Button
          variant="outline"
          size="sm"
          onClick={handleRefresh}
          disabled={refreshing}
        >
          {refreshing ? (
            <Loader2 className="h-4 w-4 animate-spin mr-1" />
          ) : (
            <RefreshCw className="h-4 w-4 mr-1" />
          )}
          Refresh
        </Button>
      </div>

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
        <LoadingSpinner label="Loading contact details..." />
      ) : !contact ? (
        /* No contact info state */
        <div className="space-y-4 text-center py-8">
          <h3 className="text-lg font-medium text-muted-foreground">
            No contact information available
          </h3>
          <p className="text-sm text-muted-foreground font-mono">
            Contact ID: {contactId}
          </p>
          <Button variant="outline" onClick={handleRefresh}>
            <RefreshCw className="h-4 w-4 mr-1" />
            Refresh from Platform
          </Button>
        </div>
      ) : (
        <div className="space-y-6">
          {/* Profile section */}
          <div className="flex items-start gap-4">
            {/* Avatar */}
            <div className="flex flex-col items-center gap-1 flex-shrink-0">
              {contact.avatarUrl ? (
                <img
                  src={contact.avatarUrl}
                  alt={`${displayName} avatar`}
                  className="h-16 w-16 rounded-full object-cover border border-border"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = "none";
                  }}
                />
              ) : (
                <div className="h-16 w-16 rounded-full bg-primary/10 flex items-center justify-center">
                  <User className="h-8 w-8 text-primary" />
                </div>
              )}
              <span className="text-xs text-muted-foreground">Contact</span>
            </div>

            {/* Info */}
            <div className="flex-1 min-w-0 space-y-1">
              <h3 className="text-xl font-semibold truncate">{displayName}</h3>
              {contact.username && (
                <p className="text-sm font-medium text-primary">
                  @{contact.username}
                </p>
              )}
              {contact.publicMessage && (
                <p className="text-sm text-muted-foreground">
                  {contact.publicMessage}
                </p>
              )}
              <div className="flex items-center gap-2 mt-2">
                <p className="text-xs text-muted-foreground font-mono truncate">
                  {contact.contactIdentityId}
                </p>
                <CopyButton value={contact.contactIdentityId} label="Copy ID" />
              </div>
            </div>

            {/* Pay button (dev mode) */}
            <Button
              variant="default"
              size="sm"
              onClick={handlePay}
              className="flex-shrink-0"
            >
              Send Payment
            </Button>
          </div>

          <Separator />

          {/* Private Contact Information */}
          <PrivateInfoSection
            privateInfo={privateInfo}
            editing={editing}
            editNickname={editNickname}
            editNote={editNote}
            editHidden={editHidden}
            saving={saving}
            showInfoPopup={showInfoPopup}
            onEditNicknameChange={setEditNickname}
            onEditNoteChange={setEditNote}
            onEditHiddenChange={setEditHidden}
            onStartEdit={handleStartEdit}
            onCancelEdit={handleCancelEdit}
            onSave={handleSave}
            onToggleInfoPopup={() => setShowInfoPopup(!showInfoPopup)}
          />

          <Separator />

          {/* Payment History */}
          <PaymentHistorySection
            payments={contactPayments}
            loading={paymentsLoading}
            currentIdentityId={selectedIdentityId}
          />

          <Separator />

          {/* Actions */}
          <div className="space-y-3">
            <h4 className="font-semibold">Actions</h4>
            <p className="text-sm text-muted-foreground">
              Contact removal and blocking are not yet available.
              Contact requests cannot be revoked once sent on Platform.
            </p>
          </div>
        </div>
      )}
    </Island>
  );
}

// ─── Private Info Section ─────────────────────────────────────────────

interface PrivateInfoSectionProps {
  privateInfo: ContactPrivateInfoDto | null;
  editing: boolean;
  editNickname: string;
  editNote: string;
  editHidden: boolean;
  saving: boolean;
  showInfoPopup: boolean;
  onEditNicknameChange: (v: string) => void;
  onEditNoteChange: (v: string) => void;
  onEditHiddenChange: (v: boolean) => void;
  onStartEdit: () => void;
  onCancelEdit: () => void;
  onSave: () => void;
  onToggleInfoPopup: () => void;
}

function PrivateInfoSection({
  privateInfo,
  editing,
  editNickname,
  editNote,
  editHidden,
  saving,
  showInfoPopup,
  onEditNicknameChange,
  onEditNoteChange,
  onEditHiddenChange,
  onStartEdit,
  onCancelEdit,
  onSave,
  onToggleInfoPopup,
}: PrivateInfoSectionProps) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <h4 className="font-semibold flex-1">Private Contact Information</h4>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              onClick={onToggleInfoPopup}
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

        {editing ? (
          <div className="flex gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={onCancelEdit}
              disabled={saving}
            >
              <X className="h-4 w-4 mr-1" />
              Cancel
            </Button>
            <Button size="sm" onClick={onSave} disabled={saving}>
              {saving ? (
                <Loader2 className="h-4 w-4 animate-spin mr-1" />
              ) : (
                <Save className="h-4 w-4 mr-1" />
              )}
              Save
            </Button>
          </div>
        ) : (
          <Button variant="outline" size="sm" onClick={onStartEdit}>
            <Pencil className="h-4 w-4 mr-1" />
            Edit
          </Button>
        )}
      </div>

      {/* Info popup dialog */}
      <Dialog open={showInfoPopup} onOpenChange={onToggleInfoPopup}>
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

      {editing ? (
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="edit-nickname">Nickname</Label>
            <Input
              id="edit-nickname"
              value={editNickname}
              onChange={(e) => onEditNicknameChange(e.target.value)}
              placeholder="Optional nickname for this contact"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-note">Notes</Label>
            <Textarea
              id="edit-note"
              value={editNote}
              onChange={(e) => onEditNoteChange(e.target.value)}
              placeholder="Private notes about this contact"
              rows={3}
            />
          </div>
          <div className="flex items-center gap-2">
            <Checkbox
              id="edit-hidden"
              checked={editHidden}
              onCheckedChange={(checked) => onEditHiddenChange(checked === true)}
            />
            <Label htmlFor="edit-hidden" className="text-sm">
              Hide this contact
            </Label>
            {editHidden && (
              <span className="text-xs text-muted-foreground">
                (Contact will not appear in lists)
              </span>
            )}
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          {privateInfo?.nickname?.trim() ? (
            <div className="flex gap-2 text-sm">
              <span className="text-muted-foreground">Nickname:</span>
              <span>{privateInfo.nickname}</span>
            </div>
          ) : null}
          {privateInfo?.notes?.trim() ? (
            <div className="space-y-1 text-sm">
              <span className="text-muted-foreground">Note:</span>
              <p className="whitespace-pre-wrap">{privateInfo.notes}</p>
            </div>
          ) : null}
          {privateInfo?.isHidden && (
            <Badge variant="outline" className="text-warning border-warning">
              This contact is hidden
            </Badge>
          )}
          {!privateInfo?.nickname?.trim() &&
            !privateInfo?.notes?.trim() &&
            !privateInfo?.isHidden && (
              <p className="text-sm text-muted-foreground italic">
                No private info set. Click Edit to add a nickname or note.
              </p>
            )}
        </div>
      )}
    </div>
  );
}

// ─── Payment History Section ──────────────────────────────────────────

interface PaymentHistorySectionProps {
  payments: StoredPaymentDto[];
  loading: boolean;
  currentIdentityId: string;
}

function PaymentHistorySection({
  payments,
  loading,
  currentIdentityId,
}: PaymentHistorySectionProps) {
  return (
    <div className="space-y-3">
      <h4 className="font-semibold">Payment History</h4>
      {loading ? (
        <LoadingSpinner label="Loading payments..." />
      ) : payments.length === 0 ? (
        <p className="text-sm text-muted-foreground italic">
          No payment history with this contact
        </p>
      ) : (
        <div className="space-y-3" role="list" aria-label="Payment history">
          {payments.map((payment) => {
            const isIncoming = payment.toIdentityId === currentIdentityId;
            return (
              <div
                key={payment.id}
                role="listitem"
                className="flex items-start gap-3 py-2"
              >
                <div className="flex-shrink-0 mt-0.5">
                  {isIncoming ? (
                    <ArrowDownLeft className="h-4 w-4 text-green-600 dark:text-green-400" />
                  ) : (
                    <ArrowUpRight className="h-4 w-4 text-destructive" />
                  )}
                </div>
                <div className="flex-1 min-w-0 space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span
                      className={cn(
                        "font-medium text-sm",
                        isIncoming
                          ? "text-green-600 dark:text-green-400"
                          : "text-destructive",
                      )}
                    >
                      {isIncoming ? "+" : "-"}
                      {formatAmount(payment.amount, 8)} Dash
                    </span>
                    {payment.createdAt > 0 && (
                      <span className="text-xs text-muted-foreground">
                        {formatRelativeTime(payment.createdAt)}
                      </span>
                    )}
                  </div>
                  {payment.memo && (
                    <p className="text-sm italic text-muted-foreground">
                      &ldquo;{payment.memo}&rdquo;
                    </p>
                  )}
                  <p className="text-xs text-muted-foreground font-mono truncate">
                    {payment.txId}
                  </p>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
