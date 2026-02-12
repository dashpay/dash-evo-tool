import { useState, useMemo, useCallback } from "react";
import {
  User,
  Pencil,
  Info,
  Loader2,
  CheckCircle,
} from "lucide-react";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback/LoadingSpinner";
import {
  CopyButton,
  ConfirmationDialog,
  WalletUnlockDialog,
} from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { formatAmount } from "@/components/shared/AmountInput";
import { cn } from "@/lib/utils";

// ─── Constants ────────────────────────────────────────────────────────

const MAX_DISPLAY_NAME = 25;
const MAX_BIO = 140;
const MAX_AVATAR_URL = 500;

const WARN_DISPLAY_NAME = 20;
const WARN_BIO = 120;
const WARN_AVATAR_URL = 450;

const PROFILE_GUIDELINES = [
  "Display names can include any UTF-8 characters (emojis, symbols, etc.).",
  "Display names are limited to 25 characters.",
  "Bios are limited to 140 characters.",
  'Avatar URLs should point to publicly accessible images (max 500 chars).',
  "Profiles are public and visible to all DashPay users.",
  "Choose a display name that represents you well — it's visible to everyone.",
  "Keep bios concise and meaningful.",
  "Inappropriate content may be flagged by the community.",
];

const AVATAR_GUIDELINES = [
  "The URL must point to a publicly accessible image.",
  "Recommended: Square images (e.g., 256×256 or 512×512 pixels).",
  "Supported formats: JPEG, PNG, WebP, or GIF.",
  "Maximum URL length: 500 characters.",
  "Example URL: https://example.com/images/avatar.jpg",
  "Tip: Use image hosting services like Imgur, Cloudinary, or your own server.",
  "The image will be center-cropped to a square if it isn't already.",
];

// ─── Validation ───────────────────────────────────────────────────────

interface ValidationError {
  field: "displayName" | "bio" | "avatarUrl";
  message: string;
}

function validateProfile(
  displayName: string,
  bio: string,
  avatarUrl: string,
): ValidationError[] {
  const errors: ValidationError[] = [];

  const trimmedName = displayName.trim();
  if (trimmedName.length === 0) {
    errors.push({
      field: "displayName",
      message: "Display name is required",
    });
  } else if (trimmedName.length > MAX_DISPLAY_NAME) {
    errors.push({
      field: "displayName",
      message: `Display name is ${trimmedName.length} characters, must be ${MAX_DISPLAY_NAME} or less`,
    });
  }

  if (bio.length > MAX_BIO) {
    errors.push({
      field: "bio",
      message: `Bio is ${bio.length} characters, must be ${MAX_BIO} or less`,
    });
  }

  if (avatarUrl.trim().length > 0) {
    if (
      !avatarUrl.trim().startsWith("http://") &&
      !avatarUrl.trim().startsWith("https://")
    ) {
      errors.push({
        field: "avatarUrl",
        message:
          "Invalid avatar URL. Must start with http:// or https://",
      });
    }
    if (avatarUrl.length > MAX_AVATAR_URL) {
      errors.push({
        field: "avatarUrl",
        message: `Avatar URL is ${avatarUrl.length} characters, must be ${MAX_AVATAR_URL} or less`,
      });
    }
  }

  return errors;
}

function counterColor(len: number, warnAt: number, maxAt: number): string {
  if (len > maxAt) return "text-destructive";
  if (len >= warnAt) return "text-warning";
  return "text-muted-foreground";
}

// ─── Profile Screen ───────────────────────────────────────────────────

export function ProfileScreen() {
  const { selectedIdentityId, profile, profileLoading, profileSaving, profileError, updateProfile, clearErrors } =
    useDashPayStore();
  const identities = useIdentityStore((s) => s.identities);
  const wallets = useWalletStore((s) => s.hdWallets);

  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  // Determine wallet alias for the unlock dialog
  const walletAlias = useMemo(() => {
    if (!selectedIdentity) return "Wallet";
    const hash = selectedIdentity.associatedWalletHashes[0];
    if (!hash) return "Wallet";
    const wallet = wallets.find((w) => w.seedHash === hash);
    return wallet?.alias ?? "Wallet";
  }, [selectedIdentity, wallets]);

  // ── Edit state ──
  const [editing, setEditing] = useState(false);
  const [editDisplayName, setEditDisplayName] = useState("");
  const [editBio, setEditBio] = useState("");
  const [editAvatarUrl, setEditAvatarUrl] = useState("");
  const [originalDisplayName, setOriginalDisplayName] = useState("");
  const [originalBio, setOriginalBio] = useState("");
  const [originalAvatarUrl, setOriginalAvatarUrl] = useState("");
  const [showSuccess, setShowSuccess] = useState(false);
  const [wasCreatingNew, setWasCreatingNew] = useState(false);
  // Tracks the identity ID we were initialized for, to reset on change
  const [trackedIdentityId, setTrackedIdentityId] = useState(selectedIdentityId);

  // ── Dialog state ──
  const [showDiscardDialog, setShowDiscardDialog] = useState(false);
  const [showWalletUnlock, setShowWalletUnlock] = useState(false);
  const [showGuidelinesSheet, setShowGuidelinesSheet] = useState(false);
  const [showAvatarGuidelinesSheet, setShowAvatarGuidelinesSheet] = useState(false);
  const [showAvatarDialog, setShowAvatarDialog] = useState(false);

  // ── Message state (success/error banners) ──
  const [message, setMessage] = useState<{ text: string; type: "success" | "error" | "info" } | null>(null);
  // Track whether we dispatched a save, so we can detect completion
  const [saveDispatched, setSaveDispatched] = useState(false);

  // Reset state when identity changes (React pattern: derive state from props)
  if (trackedIdentityId !== selectedIdentityId) {
    setTrackedIdentityId(selectedIdentityId);
    setEditing(false);
    setShowSuccess(false);
    setMessage(null);
    setSaveDispatched(false);
    clearErrors();
  }

  // Detect save completion: saveDispatched was true but store is no longer saving
  if (saveDispatched && !profileSaving) {
    setSaveDispatched(false);
    if (profileError) {
      setMessage({ text: profileError, type: "error" });
    } else {
      setEditing(false);
      setShowSuccess(true);
    }
  }

  // ── Derived values ──
  const hasUnsavedChanges = useMemo(() => {
    if (!editing) return false;
    return (
      editDisplayName !== originalDisplayName ||
      editBio !== originalBio ||
      editAvatarUrl !== originalAvatarUrl
    );
  }, [editing, editDisplayName, editBio, editAvatarUrl, originalDisplayName, originalBio, originalAvatarUrl]);

  const identityBalance = selectedIdentity?.balance ?? 0;
  const dpnsNames = selectedIdentity?.dpnsNames ?? [];
  const primaryDpnsName = dpnsNames.length > 0 ? dpnsNames[0]!.name : null;

  // ── Validation (derived, not effect-driven) ──
  const validationErrors = useMemo(
    () => (editing ? validateProfile(editDisplayName, editBio, editAvatarUrl) : []),
    [editing, editDisplayName, editBio, editAvatarUrl],
  );

  const fieldError = useCallback(
    (field: ValidationError["field"]) =>
      validationErrors.find((e) => e.field === field)?.message ?? null,
    [validationErrors],
  );

  const isValid = validationErrors.length === 0;

  // ── Handlers ──

  const startEditing = useCallback(() => {
    const dn = profile?.displayName ?? "";
    const bio = profile?.bio ?? "";
    const url = profile?.avatarUrl ?? "";
    setEditDisplayName(dn);
    setEditBio(bio);
    setEditAvatarUrl(url);
    setOriginalDisplayName(dn);
    setOriginalBio(bio);
    setOriginalAvatarUrl(url);
    setMessage(null);
    setEditing(true);
    setWasCreatingNew(!profile);
  }, [profile]);

  const cancelEditing = useCallback(() => {
    setEditing(false);
    setEditDisplayName("");
    setEditBio("");
    setEditAvatarUrl("");
    setMessage(null);
  }, []);

  const handleCancel = useCallback(() => {
    if (hasUnsavedChanges) {
      setShowDiscardDialog(true);
    } else {
      cancelEditing();
    }
  }, [hasUnsavedChanges, cancelEditing]);

  const handleSave = useCallback(() => {
    const errors = validateProfile(editDisplayName, editBio, editAvatarUrl);
    if (errors.length > 0) {
      setMessage({ text: errors[0]!.message, type: "error" });
      return;
    }
    const dn = editDisplayName.trim();
    const bio = editBio.trim();
    const url = editAvatarUrl.trim();
    setSaveDispatched(true);
    updateProfile({
      displayName: dn || null,
      bio: bio || null,
      avatarUrl: url || null,
    });
  }, [editDisplayName, editBio, editAvatarUrl, updateProfile]);

  // ── No identity selected ──
  if (!selectedIdentityId) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={User}
          title="No Identity Selected"
          description="Select an identity from the sidebar to view your DashPay profile."
        />
      </Island>
    );
  }

  // ── Loading state ──
  if (profileLoading) {
    return (
      <Island className="flex-1 flex items-center justify-center">
        <LoadingSpinner size="lg" label="Loading profile..." />
      </Island>
    );
  }

  // ── Success screen ──
  if (showSuccess) {
    return (
      <Island className="flex-1">
        <div className="flex flex-col items-center justify-center gap-6 py-20">
          <CheckCircle className="h-16 w-16 text-success" />
          <div className="text-center space-y-2">
            <h2 className="text-2xl font-bold">
              {wasCreatingNew
                ? "DashPay Profile Created Successfully!"
                : "DashPay Profile Updated Successfully!"}
            </h2>
            <p className="text-muted-foreground">
              Your profile changes have been broadcast to the network.
            </p>
          </div>
          <Button
            onClick={() => {
              setShowSuccess(false);
              setWasCreatingNew(false);
            }}
          >
            View Profile
          </Button>
        </div>
      </Island>
    );
  }

  // ── Edit mode ──
  if (editing) {
    return (
      <Island className="flex-1 overflow-y-auto">
        <div className="max-w-2xl mx-auto space-y-6">
          {/* Header */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <h2 className="text-xl font-semibold">
                {wasCreatingNew ? "Create Profile" : "Edit Profile"}
              </h2>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    onClick={() => setShowGuidelinesSheet(true)}
                    aria-label="Profile guidelines"
                  >
                    <Info className="h-4 w-4 text-muted-foreground" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Profile guidelines</TooltipContent>
              </Tooltip>
            </div>
          </div>

          {/* Display Name */}
          <div className="space-y-2">
            <Label htmlFor="profile-display-name">
              Display Name <span className="text-destructive">*</span>
            </Label>
            <Input
              id="profile-display-name"
              value={editDisplayName}
              onChange={(e) => setEditDisplayName(e.target.value)}
              placeholder="Enter your display name (required)"
              aria-invalid={!!fieldError("displayName")}
              aria-describedby="display-name-counter"
              maxLength={MAX_DISPLAY_NAME + 10} // Allow typing over for error feedback
            />
            <div className="flex items-center justify-between">
              {fieldError("displayName") ? (
                <p className="text-sm text-destructive">{fieldError("displayName")}</p>
              ) : (
                <div />
              )}
              <span
                id="display-name-counter"
                className={cn(
                  "text-xs tabular-nums",
                  counterColor(editDisplayName.length, WARN_DISPLAY_NAME, MAX_DISPLAY_NAME),
                )}
              >
                {editDisplayName.length}/{MAX_DISPLAY_NAME}
              </span>
            </div>
          </div>

          {/* Bio */}
          <div className="space-y-2">
            <Label htmlFor="profile-bio">Bio / Status</Label>
            <Textarea
              id="profile-bio"
              value={editBio}
              onChange={(e) => setEditBio(e.target.value)}
              placeholder="Tell others about yourself (optional)"
              rows={4}
              aria-invalid={!!fieldError("bio")}
              aria-describedby="bio-counter"
            />
            <div className="flex items-center justify-between">
              {fieldError("bio") ? (
                <p className="text-sm text-destructive">{fieldError("bio")}</p>
              ) : (
                <div />
              )}
              <span
                id="bio-counter"
                className={cn(
                  "text-xs tabular-nums",
                  counterColor(editBio.length, WARN_BIO, MAX_BIO),
                )}
              >
                {editBio.length}/{MAX_BIO}
              </span>
            </div>
          </div>

          {/* Avatar URL */}
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Label htmlFor="profile-avatar-url">Avatar URL</Label>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    onClick={() => setShowAvatarGuidelinesSheet(true)}
                    aria-label="Avatar guidelines"
                  >
                    <Info className="h-4 w-4 text-muted-foreground" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Avatar image guidelines</TooltipContent>
              </Tooltip>
            </div>
            <Input
              id="profile-avatar-url"
              value={editAvatarUrl}
              onChange={(e) => setEditAvatarUrl(e.target.value)}
              placeholder="https://example.com/avatar.jpg (optional)"
              aria-invalid={!!fieldError("avatarUrl")}
              aria-describedby="avatar-url-counter"
            />
            <div className="flex items-center justify-between">
              {fieldError("avatarUrl") ? (
                <p className="text-sm text-destructive">{fieldError("avatarUrl")}</p>
              ) : (
                <div />
              )}
              {editAvatarUrl.length > 0 && (
                <span
                  id="avatar-url-counter"
                  className={cn(
                    "text-xs tabular-nums",
                    counterColor(editAvatarUrl.length, WARN_AVATAR_URL, MAX_AVATAR_URL),
                  )}
                >
                  {editAvatarUrl.length}/{MAX_AVATAR_URL}
                </span>
              )}
            </div>
          </div>

          {/* Validation errors summary */}
          {validationErrors.length > 0 && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 p-4 space-y-1">
              <p className="text-sm font-medium text-destructive">Validation Errors:</p>
              <ul className="list-disc list-inside text-sm text-destructive/80 space-y-0.5">
                {validationErrors.map((err, i) => (
                  <li key={i}>{err.message}</li>
                ))}
              </ul>
            </div>
          )}

          {/* Fee estimation */}
          <div className="rounded-md border bg-muted/30 p-4">
            <p className="text-sm text-muted-foreground">
              <span className="font-medium">Estimated fee:</span>{" "}
              ~0.0001 DASH
            </p>
            <p className="text-xs text-muted-foreground mt-1">
              Identity balance: {formatAmount(identityBalance, 8)} DASH
            </p>
          </div>

          {/* Error message */}
          {message?.type === "error" && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 p-3">
              <p className="text-sm text-destructive">{message.text}</p>
            </div>
          )}

          {/* Action buttons */}
          <div className="flex items-center justify-end gap-3 pt-2">
            <Button
              variant="outline"
              onClick={handleCancel}
              disabled={profileSaving}
            >
              Cancel
            </Button>
            <Button
              onClick={handleSave}
              disabled={!isValid || profileSaving || identityBalance < 10000}
            >
              {profileSaving ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Saving…
                </>
              ) : (
                <>Save Profile</>
              )}
            </Button>
          </div>
        </div>

        {/* Discard changes confirmation */}
        <ConfirmationDialog
          open={showDiscardDialog}
          onOpenChange={setShowDiscardDialog}
          title="Discard Changes?"
          message="You have unsaved profile changes. Are you sure you want to discard them?"
          confirmText="Discard"
          cancelText="Keep Editing"
          danger
          onResult={(status) => {
            if (status === "confirmed") {
              cancelEditing();
            }
          }}
        />

        {/* Wallet unlock (placeholder for future integration) */}
        <WalletUnlockDialog
          open={showWalletUnlock}
          onOpenChange={setShowWalletUnlock}
          walletAlias={walletAlias}
          onResult={(result) => {
            if (result.status === "unlocked") {
              setShowWalletUnlock(false);
              handleSave();
            }
          }}
        />

        {/* Profile guidelines sheet */}
        <Sheet open={showGuidelinesSheet} onOpenChange={setShowGuidelinesSheet}>
          <SheetContent side="right" className="w-[400px] sm:w-[540px]">
            <SheetHeader>
              <SheetTitle>Profile Guidelines</SheetTitle>
              <SheetDescription>
                Tips for creating a great DashPay profile
              </SheetDescription>
            </SheetHeader>
            <div className="mt-6 space-y-3">
              {PROFILE_GUIDELINES.map((item, i) => (
                <div key={i} className="flex gap-2 text-sm">
                  <span className="text-muted-foreground shrink-0">•</span>
                  <span>{item}</span>
                </div>
              ))}
            </div>
          </SheetContent>
        </Sheet>

        {/* Avatar guidelines sheet */}
        <Sheet open={showAvatarGuidelinesSheet} onOpenChange={setShowAvatarGuidelinesSheet}>
          <SheetContent side="right" className="w-[400px] sm:w-[540px]">
            <SheetHeader>
              <SheetTitle>Avatar Image Guidelines</SheetTitle>
              <SheetDescription>
                Requirements for profile avatar images
              </SheetDescription>
            </SheetHeader>
            <div className="mt-6 space-y-3">
              {AVATAR_GUIDELINES.map((item, i) => (
                <div key={i} className="flex gap-2 text-sm">
                  <span className="text-muted-foreground shrink-0">•</span>
                  <span>{item}</span>
                </div>
              ))}
            </div>
          </SheetContent>
        </Sheet>
      </Island>
    );
  }

  // ── View mode — No profile ──
  if (!profile || (!profile.displayName && !profile.bio && !profile.avatarUrl)) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={User}
          title="No DashPay Profile"
          description="This identity doesn't have a DashPay profile yet. Create one to start using DashPay social features."
          actionLabel="Create Profile"
          onAction={startEditing}
        />
      </Island>
    );
  }

  // ── View mode — Profile exists ──
  return (
    <Island className="flex-1">
      <div className="max-w-2xl mx-auto space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between">
          <h2 className="text-xl font-semibold">My DashPay Profile</h2>
          <Button onClick={startEditing} size="sm">
            <Pencil className="mr-2 h-4 w-4" />
            Edit Profile
          </Button>
        </div>

        {/* Message banner */}
        {message && (
          <div
            className={cn(
              "rounded-md border p-3 text-sm",
              message.type === "success" && "border-success/30 bg-success/5 text-success",
              message.type === "error" && "border-destructive/30 bg-destructive/5 text-destructive",
              message.type === "info" && "border-info/30 bg-info/5 text-info",
            )}
            role="status"
          >
            {message.text}
          </div>
        )}

        {/* Profile error from store */}
        {profileError && !message && (
          <div className="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive" role="alert">
            {profileError}
          </div>
        )}

        {/* Profile card */}
        <div className="rounded-lg border bg-card p-6">
          <div className="flex gap-6">
            {/* Avatar */}
            <button
              type="button"
              className="shrink-0 h-20 w-20 rounded-full bg-muted flex items-center justify-center overflow-hidden border-2 border-muted hover:border-primary transition-colors cursor-pointer"
              onClick={() => profile.avatarUrl && setShowAvatarDialog(true)}
              aria-label={profile.avatarUrl ? "View avatar" : "No avatar set"}
              disabled={!profile.avatarUrl}
            >
              {profile.avatarUrl ? (
                <img
                  src={profile.avatarUrl}
                  alt={`${profile.displayName}'s avatar`}
                  className="h-full w-full object-cover"
                  onError={(e) => {
                    // Replace broken image with placeholder
                    (e.target as HTMLImageElement).style.display = "none";
                    (e.target as HTMLImageElement).parentElement!.classList.add("avatar-fallback");
                  }}
                />
              ) : (
                <User className="h-8 w-8 text-muted-foreground" />
              )}
            </button>

            {/* Profile info */}
            <div className="flex-1 min-w-0 space-y-1">
              <h3 className="text-lg font-semibold truncate">
                {profile.displayName || "Unnamed"}
              </h3>
              {primaryDpnsName && (
                <p className="text-sm text-muted-foreground">
                  @{primaryDpnsName}
                </p>
              )}
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground font-mono">
                <span className="truncate max-w-[240px]" title={selectedIdentityId}>
                  {selectedIdentityId}
                </span>
                <CopyButton value={selectedIdentityId} />
              </div>
              <div className="pt-1">
                <Badge variant="outline" className="text-xs">
                  {formatAmount(identityBalance, 8)} DASH
                </Badge>
              </div>
            </div>
          </div>

          {/* Bio section */}
          {profile.bio && (
            <div className="mt-6 pt-4 border-t">
              <p className="text-sm font-medium text-muted-foreground mb-1">Bio</p>
              <p className="text-sm whitespace-pre-wrap">{profile.bio}</p>
            </div>
          )}

          {!profile.bio && (
            <div className="mt-6 pt-4 border-t">
              <p className="text-sm text-muted-foreground italic">No bio set</p>
            </div>
          )}
        </div>

        {/* Saving indicator */}
        {profileSaving && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Saving profile...
          </div>
        )}
      </div>

      {/* Avatar display dialog */}
      <Dialog open={showAvatarDialog} onOpenChange={setShowAvatarDialog}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Profile Avatar</DialogTitle>
            <DialogDescription>
              {profile.displayName}&apos;s avatar image
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col items-center gap-4">
            <div className="h-[200px] w-[200px] rounded-lg overflow-hidden bg-muted">
              {profile.avatarUrl && (
                <img
                  src={profile.avatarUrl}
                  alt={`${profile.displayName}'s avatar`}
                  className="h-full w-full object-cover"
                />
              )}
            </div>
            {profile.avatarUrl && (
              <div className="flex items-center gap-2 w-full">
                <p className="text-xs text-muted-foreground font-mono truncate flex-1">
                  {profile.avatarUrl}
                </p>
                <CopyButton value={profile.avatarUrl} label="Copy URL" size="sm" />
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </Island>
  );
}
