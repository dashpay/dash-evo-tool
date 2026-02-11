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
import type { ContactPrivateInfoDto } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

const PUBLIC_PROFILE_INFO = [
  "This is the contact's public DashPay profile.",
  "This information is published on Dash Platform.",
  "Anyone can view this profile.",
  "The contact controls what information to share.",
  "This is different from your private notes about them.",
];

const PRIVATE_INFO_HELP = [
  "This information is encrypted and stored on Platform.",
  "It is never shared with the contact — only you can decrypt it.",
  "Only you can see these nicknames and notes.",
  "Use this to organize and remember your contacts.",
];

// ─── ContactProfileViewer ─────────────────────────────────────────────

interface ContactProfileViewerProps {
  contactId: string;
}

export function ContactProfileViewer({ contactId }: ContactProfileViewerProps) {
  const navigate = useNavigate();
  const {
    selectedIdentityId,
    contacts,
    fetchContactProfile,
    loadContactPrivateInfo,
    saveContactPrivateInfo,
    updateContactInfo,
  } = useDashPayStore();

  // ── State ──
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [privateInfo, setPrivateInfo] = useState<ContactPrivateInfoDto | null>(null);
  const [editing, setEditing] = useState(false);
  const [editNickname, setEditNickname] = useState("");
  const [editNote, setEditNote] = useState("");
  const [editHidden, setEditHidden] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ text: string; type: "success" | "error" | "info" } | null>(null);
  const [showPublicInfoPopup, setShowPublicInfoPopup] = useState(false);
  const [showPrivateInfoPopup, setShowPrivateInfoPopup] = useState(false);
  const [avatarError, setAvatarError] = useState(false);

  // ── Derived data ──
  const contact = useMemo(
    () => contacts.find((c) => c.contactIdentityId === contactId) ?? null,
    [contacts, contactId],
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

      // Fetch profile from platform if not cached
      try {
        await fetchContactProfile(contactId);
      } catch {
        // Will show "no profile" state
      }

      if (!cancelled) setLoading(false);
    }

    loadData();

    return () => {
      cancelled = true;
    };
  }, [contactId, loadContactPrivateInfo, fetchContactProfile]);

  // ── Handlers ──
  const handleBack = useCallback(() => {
    navigate({ to: "/dashpay/contacts" });
  }, [navigate]);

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
    navigate({ to: "/dashpay/send-payment/$contactId", params: { contactId } });
  }, [navigate, contactId]);

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
      await saveContactPrivateInfo({
        contactId,
        nickname: editNickname,
        notes: editNote,
        isHidden: editHidden,
      });

      await updateContactInfo({
        contactId,
        nickname: editNickname || null,
        note: editNote || null,
        isHidden: editHidden,
        acceptedAccounts: [],
      });

      setPrivateInfo({
        nickname: editNickname,
        notes: editNote,
        isHidden: editHidden,
      });

      setEditing(false);
      setMessage({ text: "Contact info saved", type: "success" });
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

  // ── Render ──
  if (!selectedIdentityId) {
    return (
      <Island>
        <EmptyState
          icon={User}
          title="No Identity Selected"
          description="Select an identity from the sidebar to view profiles."
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
        <h2 className="text-lg font-semibold flex-1">Public Profile</h2>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={() => setShowPublicInfoPopup(true)}
              aria-label="About public profiles"
            >
              <Info className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="max-w-xs">
            <ul className="space-y-1 text-xs">
              {PUBLIC_PROFILE_INFO.map((line, i) => (
                <li key={i}>• {line}</li>
              ))}
            </ul>
          </TooltipContent>
        </Tooltip>
      </div>

      {/* Public profile info dialog */}
      <Dialog open={showPublicInfoPopup} onOpenChange={setShowPublicInfoPopup}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>About Public Profiles</DialogTitle>
            <DialogDescription asChild>
              <ul className="space-y-2 mt-2">
                {PUBLIC_PROFILE_INFO.map((line, i) => (
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

      {/* Loading */}
      {loading ? (
        <LoadingSpinner label="Loading public profile..." />
      ) : (
        <div className="space-y-6">
          {contact ? (
            <>
              {/* Profile Header */}
              <div className="flex items-start gap-4">
                {/* Avatar */}
                <div className="flex flex-col items-center gap-1 flex-shrink-0">
                  {contact.avatarUrl && !avatarError ? (
                    <img
                      src={contact.avatarUrl}
                      alt={`${contact.displayName ?? "Contact"} avatar`}
                      className="h-16 w-16 rounded-lg object-cover border border-border"
                      onError={() => setAvatarError(true)}
                    />
                  ) : (
                    <div className="h-16 w-16 rounded-lg bg-primary/10 flex items-center justify-center">
                      <User className="h-8 w-8 text-primary" />
                    </div>
                  )}
                  <span className="text-xs text-muted-foreground">
                    {contact.avatarUrl && !avatarError ? "Avatar" : "No avatar"}
                  </span>
                </div>

                {/* Info */}
                <div className="flex-1 min-w-0 space-y-1">
                  {contact.displayName?.trim() ? (
                    <h3 className="text-xl font-semibold truncate">
                      {contact.displayName}
                    </h3>
                  ) : (
                    <h3 className="text-xl font-semibold italic text-muted-foreground">
                      No display name set
                    </h3>
                  )}

                  <div className="flex items-center gap-2">
                    <p className="text-xs text-muted-foreground font-mono truncate">
                      Identity: {contact.contactIdentityId}
                    </p>
                    <CopyButton value={contact.contactIdentityId} label="Copy ID" />
                  </div>

                  <div className="mt-2">
                    <p className="text-sm font-medium">Public Message:</p>
                    {contact.publicMessage?.trim() ? (
                      <p className="text-sm text-foreground">
                        {contact.publicMessage}
                      </p>
                    ) : (
                      <p className="text-sm italic text-muted-foreground">
                        No public message
                      </p>
                    )}
                  </div>
                </div>
              </div>

              {/* Avatar verification — show if avatarUrl exists */}
              {contact.avatarUrl && (
                <div className="space-y-2">
                  <h4 className="text-sm font-semibold text-muted-foreground">
                    Avatar Verification
                  </h4>
                  <div className="space-y-1 text-xs text-muted-foreground font-mono">
                    <p>URL: {contact.avatarUrl}</p>
                  </div>
                </div>
              )}

              {/* Action buttons */}
              <div className="flex gap-2">
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
                <Button variant="default" size="sm" onClick={handlePay}>
                  Pay
                </Button>
              </div>
            </>
          ) : (
            /* No profile state */
            <div className="space-y-4 text-center py-8">
              <h3 className="text-lg font-medium text-muted-foreground">
                No profile found
              </h3>
              <p className="text-sm text-muted-foreground">
                This contact has not created a public profile yet.
              </p>
              <div className="flex gap-2 justify-center">
                <Button variant="outline" onClick={handleRefresh} disabled={refreshing}>
                  {refreshing ? (
                    <Loader2 className="h-4 w-4 animate-spin mr-1" />
                  ) : (
                    <RefreshCw className="h-4 w-4 mr-1" />
                  )}
                  Retry
                </Button>
                <Button variant="default" onClick={handlePay}>
                  Pay
                </Button>
              </div>
            </div>
          )}

          <Separator />

          {/* Private Contact Information */}
          <PrivateInfoSection
            privateInfo={privateInfo}
            editing={editing}
            editNickname={editNickname}
            editNote={editNote}
            editHidden={editHidden}
            saving={saving}
            showInfoPopup={showPrivateInfoPopup}
            onEditNicknameChange={setEditNickname}
            onEditNoteChange={setEditNote}
            onEditHiddenChange={setEditHidden}
            onStartEdit={handleStartEdit}
            onCancelEdit={handleCancelEdit}
            onSave={handleSave}
            onToggleInfoPopup={() => setShowPrivateInfoPopup(!showPrivateInfoPopup)}
          />
        </div>
      )}
    </Island>
  );
}

// ─── Private Info Section (shared pattern) ────────────────────────────

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
            <DialogTitle>Private Contact Information</DialogTitle>
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
            <Label htmlFor="edit-notes">Notes</Label>
            <Textarea
              id="edit-notes"
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
              Hide this contact from the main list
            </Label>
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="flex gap-2 text-sm">
            <span className="text-muted-foreground">Nickname:</span>
            <span>
              {privateInfo?.nickname?.trim() ? (
                privateInfo.nickname
              ) : (
                <span className="italic text-muted-foreground">Not set</span>
              )}
            </span>
          </div>
          <div className="space-y-1 text-sm">
            <span className="text-muted-foreground">Notes:</span>
            {privateInfo?.notes?.trim() ? (
              <p className="whitespace-pre-wrap">{privateInfo.notes}</p>
            ) : (
              <p className="italic text-muted-foreground">No notes</p>
            )}
          </div>
          <div className="flex gap-2 text-sm">
            <span className="text-muted-foreground">Hidden:</span>
            <span>{privateInfo?.isHidden ? "Yes" : "No"}</span>
          </div>
        </div>
      )}
    </div>
  );
}
