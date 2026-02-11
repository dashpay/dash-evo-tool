import { useState, useMemo, useCallback } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  Search,
  Users,
  UserPlus,
  Eye,
  EyeOff,
  User,
  ChevronDown,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { Island } from "@/components/layout/Island";
import { EmptyState } from "@/components/feedback/EmptyState";
import { LoadingSpinner } from "@/components/feedback/LoadingSpinner";
import { ContactRequests } from "@/components/dashpay/ContactRequests";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";
import { useDashPayStore } from "@/stores/dashpayStore";
import type { StoredContactDto } from "@/bindings";
import type { ContactFilter, ContactSortField } from "@/stores/dashpayStore";

// ─── Filter / Sort labels ────────────────────────────────────────────

const FILTER_LABELS: Record<ContactFilter, string> = {
  all: "All",
  withUsernames: "With usernames",
  noUsernames: "No usernames",
  withBio: "With bio",
  recent: "Recent (7d)",
  hidden: "Hidden",
  visible: "Visible",
};

const SORT_LABELS: Record<ContactSortField, string> = {
  name: "Name",
  username: "Username",
  date: "Date Added",
  account: "Account",
};

// ─── Helpers ─────────────────────────────────────────────────────────

function getContactDisplayName(contact: StoredContactDto): string {
  return (
    contact.displayName?.trim() ||
    contact.username?.trim() ||
    `Contact (${contact.contactIdentityId.slice(0, 8)})`
  );
}

function isContactHidden(contact: StoredContactDto): boolean {
  return contact.contactStatus === "hidden";
}

/** Check if a contact was created within the last 7 days */
function isRecent(contact: StoredContactDto): boolean {
  const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000;
  const now = Date.now();
  return now - contact.createdAt * 1000 < SEVEN_DAYS_MS;
}

/** Apply filter predicate to a contact */
function matchesFilter(
  contact: StoredContactDto,
  filter: ContactFilter,
  showHidden: boolean,
): boolean {
  const hidden = isContactHidden(contact);

  switch (filter) {
    case "all":
      return hidden ? showHidden : true;
    case "withUsernames":
      if (!contact.username) return false;
      return hidden ? showHidden : true;
    case "noUsernames":
      if (contact.username) return false;
      return hidden ? showHidden : true;
    case "withBio":
      if (!contact.publicMessage) return false;
      return hidden ? showHidden : true;
    case "recent":
      if (!isRecent(contact)) return false;
      return hidden ? showHidden : true;
    case "hidden":
      return hidden;
    case "visible":
      return !hidden;
  }
}

/** Check if a contact matches the search query */
function matchesSearch(contact: StoredContactDto, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    (contact.username?.toLowerCase().includes(q) ?? false) ||
    (contact.displayName?.toLowerCase().includes(q) ?? false) ||
    (contact.publicMessage?.toLowerCase().includes(q) ?? false) ||
    contact.contactIdentityId.toLowerCase().includes(q)
  );
}

/** Sort contacts by the given field and order */
function sortContacts(
  contacts: StoredContactDto[],
  field: ContactSortField,
  order: "ascending" | "descending",
): StoredContactDto[] {
  const sorted = [...contacts].sort((a, b) => {
    let cmp = 0;
    switch (field) {
      case "name": {
        const aName = getContactDisplayName(a).toLowerCase();
        const bName = getContactDisplayName(b).toLowerCase();
        cmp = aName.localeCompare(bName);
        break;
      }
      case "username": {
        const aUser = (a.username ?? "zzz").toLowerCase();
        const bUser = (b.username ?? "zzz").toLowerCase();
        cmp = aUser.localeCompare(bUser);
        break;
      }
      case "date":
        // Newest first by default in ascending
        cmp = b.createdAt - a.createdAt;
        break;
      case "account":
        // No account reference in StoredContactDto, sort by ID
        cmp = a.contactIdentityId.localeCompare(b.contactIdentityId);
        break;
    }
    return order === "descending" ? -cmp : cmp;
  });
  return sorted;
}

// ─── ContactCard component ───────────────────────────────────────────

interface ContactCardProps {
  contact: StoredContactDto;
  onViewProfile: (contactId: string) => void;
  onPay: (contactId: string) => void;
  onToggleHidden: (contactId: string, isHidden: boolean) => void;
}

function ContactCard({
  contact,
  onViewProfile,
  onPay,
  onToggleHidden,
}: ContactCardProps) {
  const hidden = isContactHidden(contact);
  const displayName = getContactDisplayName(contact);

  return (
    <div
      className={cn(
        "flex items-center gap-4 rounded-lg border bg-card p-4 transition-colors hover:bg-muted/30",
        hidden && "opacity-70",
      )}
      data-testid="contact-card"
    >
      {/* Avatar */}
      <div className="shrink-0 h-10 w-10 rounded-full bg-muted flex items-center justify-center overflow-hidden">
        {contact.avatarUrl ? (
          <img
            src={contact.avatarUrl}
            alt={`${displayName}'s avatar`}
            className="h-full w-full object-cover"
            onError={(e) => {
              (e.target as HTMLImageElement).style.display = "none";
            }}
          />
        ) : (
          <User className="h-5 w-5 text-muted-foreground" />
        )}
      </div>

      {/* Contact info */}
      <div className="flex-1 min-w-0 space-y-0.5">
        <p className="text-sm font-medium truncate">
          {hidden && (
            <span className="text-muted-foreground">[Hidden] </span>
          )}
          {displayName}
        </p>
        {contact.username &&
          contact.username !== contact.displayName && (
            <p className="text-xs text-muted-foreground truncate">
              @{contact.username}
            </p>
          )}
        {contact.publicMessage && (
          <p className="text-xs text-muted-foreground truncate max-w-[300px]">
            {contact.publicMessage}
          </p>
        )}
      </div>

      {/* Actions */}
      <div className="flex items-center gap-2 shrink-0">
        <Button
          variant="ghost"
          size="sm"
          onClick={() =>
            onToggleHidden(contact.contactIdentityId, !hidden)
          }
          title={hidden ? "Unhide contact" : "Hide contact"}
        >
          {hidden ? (
            <Eye className="mr-1 h-3.5 w-3.5" />
          ) : (
            <EyeOff className="mr-1 h-3.5 w-3.5" />
          )}
          {hidden ? "Unhide" : "Hide"}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => onPay(contact.contactIdentityId)}
        >
          Pay
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={() => onViewProfile(contact.contactIdentityId)}
        >
          View Profile
        </Button>
      </div>
    </div>
  );
}

// ─── Re-export ContactRequests (inline) ──────────────────────────────
// The full ContactRequests component is in components/dashpay/ContactRequests.tsx

// ─── ContactsListScreen ──────────────────────────────────────────────

export function ContactsListScreen() {
  const navigate = useNavigate();

  // Store state
  const contacts = useDashPayStore((s) => s.contacts);
  const contactsLoading = useDashPayStore((s) => s.contactsLoading);
  const contactsRefreshing = useDashPayStore((s) => s.contactsRefreshing);
  const contactsError = useDashPayStore((s) => s.contactsError);
  const contactSearchQuery = useDashPayStore((s) => s.contactSearchQuery);
  const contactFilter = useDashPayStore((s) => s.contactFilter);
  const contactSortField = useDashPayStore((s) => s.contactSortField);
  const contactSortOrder = useDashPayStore((s) => s.contactSortOrder);
  const showHidden = useDashPayStore((s) => s.showHidden);
  const selectedIdentityId = useDashPayStore((s) => s.selectedIdentityId);
  const incomingRequests = useDashPayStore((s) => s.incomingRequests);

  // Store actions
  const setContactSearchQuery = useDashPayStore(
    (s) => s.setContactSearchQuery,
  );
  const setContactFilter = useDashPayStore((s) => s.setContactFilter);
  const setContactSortField = useDashPayStore((s) => s.setContactSortField);
  const toggleShowHidden = useDashPayStore((s) => s.toggleShowHidden);
  const setContactHidden = useDashPayStore((s) => s.setContactHidden);
  const refreshContacts = useDashPayStore((s) => s.refreshContacts);

  // Local state
  const [activeTab, setActiveTab] = useState<"contacts" | "requests">(
    "contacts",
  );

  // Filtered and sorted contacts
  const filteredContacts = useMemo(() => {
    const filtered = contacts.filter(
      (c) =>
        matchesFilter(c, contactFilter, showHidden) &&
        matchesSearch(c, contactSearchQuery),
    );
    return sortContacts(filtered, contactSortField, contactSortOrder);
  }, [
    contacts,
    contactFilter,
    showHidden,
    contactSearchQuery,
    contactSortField,
    contactSortOrder,
  ]);

  // Handlers
  const handleViewProfile = useCallback(
    (contactId: string) => {
      // Will be wired to ContactProfileViewer screen when implemented
      void contactId;
    },
    [],
  );

  const handlePay = useCallback(
    (contactId: string) => {
      // Will be wired to SendPayment screen when implemented
      void contactId;
    },
    [],
  );

  const handleToggleHidden = useCallback(
    (contactId: string, isHidden: boolean) => {
      setContactHidden(contactId, isHidden);
    },
    [setContactHidden],
  );

  const handleAddContact = useCallback(() => {
    navigate({ to: "/dashpay/add-contact" });
  }, [navigate]);

  const handleRefresh = useCallback(() => {
    refreshContacts();
  }, [refreshContacts]);

  const pendingRequestCount = incomingRequests.filter(
    (r) => r.status === "pending",
  ).length;

  // ── No identity selected ──
  if (!selectedIdentityId) {
    return (
      <Island className="flex-1">
        <EmptyState
          icon={Users}
          title="No Identity Selected"
          description="Select an identity from the sidebar to view contacts."
        />
      </Island>
    );
  }

  // ── Loading state ──
  if (contactsLoading) {
    return (
      <Island className="flex-1 flex items-center justify-center">
        <LoadingSpinner size="lg" label="Loading contacts..." />
      </Island>
    );
  }

  return (
    <Island className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="flex items-center justify-between px-1 pb-4">
        <h2 className="text-xl font-semibold">Contacts</h2>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={contactsRefreshing}
          >
            {contactsRefreshing ? (
              <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
            )}
            Refresh
          </Button>
          <Button size="sm" onClick={handleAddContact}>
            <UserPlus className="mr-1.5 h-3.5 w-3.5" />
            Add Contact
          </Button>
        </div>
      </div>

      {/* Tabs */}
      <Tabs
        value={activeTab}
        onValueChange={(v) => setActiveTab(v as "contacts" | "requests")}
      >
        <TabsList className="mb-4">
          <TabsTrigger value="contacts">My Contacts</TabsTrigger>
          <TabsTrigger value="requests">
            Requests
            {pendingRequestCount > 0 && (
              <Badge
                variant="secondary"
                className="ml-1.5 h-5 min-w-[20px] rounded-full px-1.5 text-xs"
              >
                {pendingRequestCount}
              </Badge>
            )}
          </TabsTrigger>
        </TabsList>
      </Tabs>

      {/* Error banner */}
      {contactsError && (
        <div
          className="rounded-md border border-destructive/30 bg-destructive/5 p-3 mb-4 text-sm text-destructive"
          role="alert"
        >
          {contactsError}
        </div>
      )}

      {/* Tab content */}
      {activeTab === "requests" ? (
        <ContactRequests onAddContact={handleAddContact} />
      ) : (
        <ContactsTabContent
          contacts={contacts}
          filteredContacts={filteredContacts}
          searchQuery={contactSearchQuery}
          filter={contactFilter}
          sortField={contactSortField}
          showHidden={showHidden}
          onSearchChange={setContactSearchQuery}
          onFilterChange={setContactFilter}
          onSortChange={setContactSortField}
          onToggleShowHidden={toggleShowHidden}
          onViewProfile={handleViewProfile}
          onPay={handlePay}
          onToggleHidden={handleToggleHidden}
          onAddContact={handleAddContact}
        />
      )}
    </Island>
  );
}

// ─── Contacts Tab Content ────────────────────────────────────────────

interface ContactsTabContentProps {
  contacts: StoredContactDto[];
  filteredContacts: StoredContactDto[];
  searchQuery: string;
  filter: ContactFilter;
  sortField: ContactSortField;
  showHidden: boolean;
  onSearchChange: (query: string) => void;
  onFilterChange: (filter: ContactFilter) => void;
  onSortChange: (field: ContactSortField) => void;
  onToggleShowHidden: () => void;
  onViewProfile: (contactId: string) => void;
  onPay: (contactId: string) => void;
  onToggleHidden: (contactId: string, isHidden: boolean) => void;
  onAddContact: () => void;
}

function ContactsTabContent({
  contacts,
  filteredContacts,
  searchQuery,
  filter,
  sortField,
  showHidden,
  onSearchChange,
  onFilterChange,
  onSortChange,
  onToggleShowHidden,
  onViewProfile,
  onPay,
  onToggleHidden,
  onAddContact,
}: ContactsTabContentProps) {
  // No contacts at all — empty state
  if (contacts.length === 0) {
    return (
      <EmptyState
        icon={Users}
        title="No Contacts"
        description="You haven't added any contacts yet."
        actionLabel="Add Contact"
        onAction={onAddContact}
      />
    );
  }

  return (
    <div className="flex flex-col flex-1 min-h-0">
      {/* Search / Filter / Sort controls */}
      <div className="flex items-center gap-3 flex-wrap pb-4">
        {/* Search */}
        <div className="relative flex-1 min-w-[200px] max-w-[300px]">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder="Search contacts..."
            className="pl-9 h-9"
            aria-label="Search contacts"
          />
          {searchQuery && (
            <button
              type="button"
              className="absolute right-2 top-2 text-xs text-muted-foreground hover:text-foreground"
              onClick={() => onSearchChange("")}
              aria-label="Clear search"
            >
              Clear
            </button>
          )}
        </div>

        {/* Filter dropdown */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="h-9">
              Filter: {FILTER_LABELS[filter]}
              <ChevronDown className="ml-1.5 h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            {(Object.keys(FILTER_LABELS) as ContactFilter[]).map((f) => (
              <DropdownMenuItem
                key={f}
                onClick={() => onFilterChange(f)}
                className={cn(f === filter && "bg-accent")}
              >
                {FILTER_LABELS[f]}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>

        {/* Sort dropdown */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="h-9">
              Sort: {SORT_LABELS[sortField]}
              <ChevronDown className="ml-1.5 h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            {(Object.keys(SORT_LABELS) as ContactSortField[]).map((s) => (
              <DropdownMenuItem
                key={s}
                onClick={() => onSortChange(s)}
                className={cn(s === sortField && "bg-accent")}
              >
                {SORT_LABELS[s]}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>

        {/* Show hidden checkbox */}
        <div className="flex items-center gap-2">
          <Checkbox
            id="show-hidden"
            checked={showHidden}
            onCheckedChange={onToggleShowHidden}
          />
          <label
            htmlFor="show-hidden"
            className="text-sm text-muted-foreground cursor-pointer select-none"
          >
            Show hidden
          </label>
        </div>
      </div>

      <Separator className="mb-4" />

      {/* Contact list or filtered empty state */}
      {filteredContacts.length === 0 ? (
        <EmptyState
          icon={Search}
          title="No Matches"
          description="No contacts match your current search or filter."
        />
      ) : (
        <div
          className="flex-1 overflow-y-auto space-y-2"
          role="list"
          aria-label="Contact list"
        >
          {filteredContacts.map((contact) => (
            <div role="listitem" key={contact.contactIdentityId}>
              <ContactCard
                contact={contact}
                onViewProfile={onViewProfile}
                onPay={onPay}
                onToggleHidden={onToggleHidden}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
