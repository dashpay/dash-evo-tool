import { useState, useCallback, useEffect } from "react";
import {
  Search,
  X,
  UserPlus,
  Eye,
  Info,
  Loader2,
  SearchX,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback/LoadingSpinner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
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

// ─── Constants ────────────────────────────────────────────────────────

const PROFILE_SEARCH_INFO = [
  "Search for users by their DPNS username.",
  "Usernames are unique, verified identifiers on Dash Platform.",
  "Results show the username along with profile info (if available).",
  "Add contacts directly from search results.",
];

const PUBLIC_MESSAGE_MAX_DISPLAY = 60;

// ─── Component ────────────────────────────────────────────────────────

export function ProfileSearchScreen() {
  const navigate = useNavigate();
  const {
    selectedIdentityId,
    searchResults,
    searchLoading,
    searchError,
    searchProfiles,
    clearSearchResults,
  } = useDashPayStore();

  const [searchQuery, setSearchQuery] = useState("");
  const [hasSearched, setHasSearched] = useState(false);
  const [showInfoPopup, setShowInfoPopup] = useState(false);

  // Show toast when search error occurs
  useEffect(() => {
    if (searchError) {
      toastError(searchError);
    }
  }, [searchError]);

  // ── Handlers ─────────────────────────────────────────────────────

  const handleSearch = useCallback(() => {
    const trimmed = searchQuery.trim();
    if (!trimmed) return;
    setHasSearched(true);
    searchProfiles(trimmed);
  }, [searchQuery, searchProfiles]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleSearch();
      }
    },
    [handleSearch],
  );

  const handleClear = useCallback(() => {
    setSearchQuery("");
    setHasSearched(false);
    clearSearchResults();
  }, [clearSearchResults]);

  const handleViewProfile = useCallback(
    (identityId: string) => {
      if (!selectedIdentityId) return;
      navigate({
        to: "/dashpay/contact-profile/$contactId",
        params: { contactId: identityId },
      });
    },
    [navigate, selectedIdentityId],
  );

  const handleAddContact = useCallback(
    (identityId: string) => {
      const result = searchResults.find((r) => r.identityId === identityId);
      navigate({
        to: "/dashpay/add-contact",
        search: { username: result?.username },
      });
    },
    [navigate, searchResults],
  );

  // ── Render ───────────────────────────────────────────────────────

  if (!selectedIdentityId) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={Search}
          title="No Identity Selected"
          description="Select an identity from the DashPay header to search for profiles."
        />
      </Island>
    );
  }

  return (
    <Island className="flex-1">
      <div className="flex flex-col gap-6">
        {/* Header */}
        <div className="flex items-center justify-between">
          <h2 className="text-xl font-semibold text-foreground">
            Search Public Profiles
          </h2>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => setShowInfoPopup(true)}
                aria-label="About profile search"
              >
                <Info className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>About Profile Search</TooltipContent>
          </Tooltip>
        </div>

        {/* Error display */}
        {searchError && (
          <div
            className="rounded-lg border border-destructive/50 bg-destructive/10 px-4 py-3 text-sm text-destructive"
            role="alert"
          >
            {searchError}
          </div>
        )}

        {/* Search input section */}
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <Input
              placeholder="Enter DPNS username..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={handleKeyDown}
              className="max-w-md"
              aria-label="DPNS username search"
            />
            <Button
              onClick={handleSearch}
              disabled={searchLoading || !searchQuery.trim()}
            >
              {searchLoading ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Search className="mr-2 h-4 w-4" />
              )}
              Search
            </Button>
            {(hasSearched || searchQuery) && (
              <Button variant="outline" onClick={handleClear}>
                <X className="mr-2 h-4 w-4" />
                Clear
              </Button>
            )}
          </div>
          <p className="text-xs text-muted-foreground">
            Tip: Search by DPNS username prefix (e.g., &quot;john&quot; finds
            &quot;john.dash&quot;, &quot;johnny.dash&quot;, etc.)
          </p>
        </div>

        {/* Loading state */}
        {searchLoading && (
          <div className="flex items-center justify-center py-12">
            <LoadingSpinner label="Searching..." />
          </div>
        )}

        {/* Results section */}
        {!searchLoading && searchResults.length > 0 && (
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-medium text-muted-foreground">
                Search Results
              </h3>
              <Badge variant="secondary">{searchResults.length}</Badge>
            </div>

            <div className="flex flex-col gap-2">
              {searchResults.map((result) => (
                <div
                  key={result.identityId}
                  className={cn(
                    "flex items-start justify-between rounded-lg border p-4",
                    "bg-card hover:bg-accent/50 transition-colors",
                  )}
                >
                  {/* Left: profile info */}
                  <div className="flex flex-col gap-1 min-w-0 flex-1">
                    {/* Username */}
                    <span className="text-base font-semibold text-foreground">
                      {result.username}
                    </span>

                    {/* Display name */}
                    {result.displayName && (
                      <span className="text-sm text-muted-foreground">
                        {result.displayName}
                      </span>
                    )}

                    {/* Public message preview */}
                    {result.publicMessage && (
                      <span className="text-xs italic text-muted-foreground truncate max-w-md">
                        {result.publicMessage.length > PUBLIC_MESSAGE_MAX_DISPLAY
                          ? `${result.publicMessage.slice(0, PUBLIC_MESSAGE_MAX_DISPLAY)}...`
                          : result.publicMessage}
                      </span>
                    )}

                    {/* Identity ID */}
                    <span className="text-xs font-mono text-muted-foreground/70">
                      ID: {result.identityId}
                    </span>
                  </div>

                  {/* Right: action buttons */}
                  <div className="flex items-center gap-2 ml-4 shrink-0">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => handleViewProfile(result.identityId)}
                        >
                          <Eye className="mr-1 h-3.5 w-3.5" />
                          View Profile
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>View public profile</TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="default"
                          size="sm"
                          onClick={() => handleAddContact(result.identityId)}
                        >
                          <UserPlus className="mr-1 h-3.5 w-3.5" />
                          Add Contact
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>Send contact request</TooltipContent>
                    </Tooltip>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* No results state */}
        {!searchLoading && hasSearched && searchResults.length === 0 && !searchError && (
          <div className="flex items-center justify-center py-12">
            <EmptyState
              icon={SearchX}
              title="No Users Found"
              description="Try a different username or check the spelling."
            />
          </div>
        )}
      </div>

      {/* Info popup */}
      <Dialog open={showInfoPopup} onOpenChange={setShowInfoPopup}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>About Profile Search</DialogTitle>
            <DialogDescription>
              Learn how profile search works on Dash Platform.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-3 text-sm text-foreground">
            {PROFILE_SEARCH_INFO.map((text, i) => (
              <p key={i}>{text}</p>
            ))}
          </div>
        </DialogContent>
      </Dialog>
    </Island>
  );
}
