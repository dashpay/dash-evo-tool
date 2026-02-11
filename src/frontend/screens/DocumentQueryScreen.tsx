import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Search,
  Plus,
  FileUp,
  FilePen,
  FileX2,
  FilePlus2,
  FileOutput,
  ShoppingCart,
  DollarSign,
  Users,
  ChevronLeft,
  ChevronRight,
  Loader2,
  SlidersHorizontal,
  X,
  FileQuestion,
  Filter,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Island, PageHeader } from "@/components/layout";
import { EmptyState } from "@/components/feedback";
import { ContractTreePanel } from "@/components/contract/ContractTreePanel";
import type { TreeSelection, IndexInfo } from "@/components/contract/ContractTreePanel";
import { useContractStore } from "@/stores/contractStore";
import { useDocumentStore, DOCUMENT_PRIVATE_FIELDS } from "@/stores/documentStore";
import type { DocumentPageEntry, DocumentDisplayMode } from "@/stores/documentStore";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";

// ─── Helpers ─────────────────────────────────────────────────────────

/** Filter a document's data object to only include selected fields. */
function filterDocumentFields(
  entry: DocumentPageEntry,
  fieldSelection: Record<string, boolean>,
): Record<string, unknown> | null {
  if (!entry.document) return null;
  const data = entry.document.data;
  if (!data || typeof data !== "object" || Array.isArray(data)) return null;

  const obj = data as Record<string, unknown>;
  const filtered: Record<string, unknown> = {};

  for (const [field, visible] of Object.entries(fieldSelection)) {
    if (!visible) continue;

    // System fields come from document metadata, not data
    if (field === "$id") {
      filtered.$id = entry.document.id;
    } else if (field === "$ownerId") {
      filtered.$ownerId = entry.document.ownerId;
    } else if (field === "$version" || field === "$revision") {
      filtered[field] = entry.document.revision;
    } else if (field === "$createdAt") {
      filtered.$createdAt = entry.document.createdAt;
    } else if (field === "$updatedAt") {
      filtered.$updatedAt = entry.document.updatedAt;
    } else if (field === "$transferredAt") {
      filtered.$transferredAt = entry.document.transferredAt;
    } else if (field.startsWith("$")) {
      // Other system fields — check document data
      if (field in obj) {
        filtered[field] = obj[field];
      }
    } else if (field in obj) {
      filtered[field] = obj[field];
    }
  }

  return filtered;
}

/** Format a document entry as JSON or YAML string. */
function formatDocument(
  filtered: Record<string, unknown>,
  mode: DocumentDisplayMode,
): string {
  if (mode === "json") {
    return JSON.stringify(filtered, null, 2);
  }
  // Simple YAML formatting
  return toSimpleYaml(filtered, 0);
}

/** Very basic YAML-like output for nested objects. */
function toSimpleYaml(value: unknown, indent: number): string {
  const prefix = "  ".repeat(indent);
  if (value === null || value === undefined) return `${prefix}null`;
  if (typeof value === "string") return `${prefix}${JSON.stringify(value)}`;
  if (typeof value === "number" || typeof value === "boolean")
    return `${prefix}${value}`;
  if (Array.isArray(value)) {
    if (value.length === 0) return `${prefix}[]`;
    return value
      .map((item) => `${prefix}- ${toSimpleYaml(item, 0).trim()}`)
      .join("\n");
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) return `${prefix}{}`;
    return entries
      .map(([k, v]) => {
        if (v === null || v === undefined || typeof v !== "object") {
          return `${prefix}${k}: ${toSimpleYaml(v, 0).trim()}`;
        }
        return `${prefix}${k}:\n${toSimpleYaml(v, indent + 1)}`;
      })
      .join("\n");
  }
  return `${prefix}${String(value)}`;
}

// ─── Action Toolbar Buttons ──────────────────────────────────────────

interface ActionButton {
  label: string;
  icon: React.ReactNode;
  route?: string;
  onClick?: () => void;
}

function useActionButtons(): ActionButton[] {
  return useMemo(() => [
    {
      label: "Load Contracts",
      icon: <Plus className="h-4 w-4" />,
      route: "/contracts/add-contracts",
    },
    {
      label: "Register Contract",
      icon: <FileUp className="h-4 w-4" />,
      route: "/contracts/register",
    },
    {
      label: "Update Contract",
      icon: <FilePen className="h-4 w-4" />,
      route: "/contracts/update-contract",
    },
    {
      label: "Create Document",
      icon: <FilePlus2 className="h-4 w-4" />,
      route: "/contracts/create-document",
    },
    {
      label: "Delete Document",
      icon: <FileX2 className="h-4 w-4" />,
      route: "/contracts/delete-document",
    },
    {
      label: "Replace Document",
      icon: <FileOutput className="h-4 w-4" />,
      route: "/contracts/replace-document",
    },
    {
      label: "Transfer Document",
      icon: <FileOutput className="h-4 w-4" />,
      route: "/contracts/transfer-document",
    },
    {
      label: "Purchase Document",
      icon: <ShoppingCart className="h-4 w-4" />,
      route: "/contracts/purchase-document",
    },
    {
      label: "Set Document Price",
      icon: <DollarSign className="h-4 w-4" />,
      route: "/contracts/set-document-price",
    },
    {
      label: "Group Actions",
      icon: <Users className="h-4 w-4" />,
      route: "/contracts/group-actions",
    },
  ], []);
}

// ─── Field Selection Dialog ──────────────────────────────────────────

interface FieldSelectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  fieldSelection: Record<string, boolean>;
  onToggleField: (field: string) => void;
  onSelectAll: () => void;
  onDeselectAll: () => void;
}

function FieldSelectionDialog({
  open,
  onOpenChange,
  fieldSelection,
  onToggleField,
  onSelectAll,
  onDeselectAll,
}: FieldSelectionDialogProps) {
  const privateFieldSet = useMemo(
    () => new Set<string>(DOCUMENT_PRIVATE_FIELDS),
    [],
  );

  const { docFields, systemFields } = useMemo(() => {
    const doc: [string, boolean][] = [];
    const sys: [string, boolean][] = [];
    for (const [field, visible] of Object.entries(fieldSelection)) {
      if (privateFieldSet.has(field)) {
        sys.push([field, visible]);
      } else {
        doc.push([field, visible]);
      }
    }
    doc.sort(([a], [b]) => a.localeCompare(b));
    sys.sort(([a], [b]) => a.localeCompare(b));
    return { docFields: doc, systemFields: sys };
  }, [fieldSelection, privateFieldSet]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg" data-testid="field-selection-dialog">
        <DialogHeader>
          <DialogTitle>Select Properties</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          Check the properties to display in query results:
        </p>
        <div className="grid grid-cols-2 gap-6 mt-4 max-h-80 overflow-y-auto">
          {/* Document properties column */}
          <div>
            <h4 className="text-sm font-semibold mb-2">Document Properties</h4>
            <div className="space-y-2">
              {docFields.map(([field, visible]) => (
                <div key={field} className="flex items-center gap-2">
                  <Checkbox
                    id={`field-${field}`}
                    checked={visible}
                    onCheckedChange={() => onToggleField(field)}
                    data-testid={`field-checkbox-${field}`}
                  />
                  <Label
                    htmlFor={`field-${field}`}
                    className="text-sm font-mono cursor-pointer"
                  >
                    {field}
                  </Label>
                </div>
              ))}
              {docFields.length === 0 && (
                <p className="text-xs text-muted-foreground">No schema properties</p>
              )}
            </div>
          </div>
          {/* System properties column */}
          <div>
            <h4 className="text-sm font-semibold mb-2">Universal Properties</h4>
            <div className="space-y-2">
              {systemFields.map(([field, visible]) => (
                <div key={field} className="flex items-center gap-2">
                  <Checkbox
                    id={`field-${field}`}
                    checked={visible}
                    onCheckedChange={() => onToggleField(field)}
                    data-testid={`field-checkbox-${field}`}
                  />
                  <Label
                    htmlFor={`field-${field}`}
                    className="text-sm font-mono cursor-pointer"
                  >
                    {field}
                  </Label>
                </div>
              ))}
            </div>
          </div>
        </div>
        <DialogFooter className="flex gap-2 sm:justify-between">
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={onSelectAll} data-testid="select-all-fields">
              Select All
            </Button>
            <Button variant="outline" size="sm" onClick={onDeselectAll} data-testid="deselect-all-fields">
              Deselect All
            </Button>
          </div>
          <Button onClick={() => onOpenChange(false)} data-testid="close-field-dialog">
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ─── Main Screen ─────────────────────────────────────────────────────

export function DocumentQueryScreen() {
  const navigate = useNavigate();
  // Contract store
  const {
    contracts,
    selectedContractId,
    selectedContractDetail,
    loading: contractsLoading,
    loadContracts,
    selectContract,
    removeContract,
    subscribeToUpdates: subscribeContractUpdates,
  } = useContractStore();

  // Document store
  const {
    queryText,
    documents,
    queryStatus,
    queryStartedAt,
    queryError,
    displayMode,
    searchFilter,
    fieldSelection,
    currentPage,
    hasNextPage,
    queryContractId,
    queryDocumentType,
    setQueryText,
    fetchDocuments,
    goToNextPage,
    goToPreviousPage,
    setDisplayMode,
    setSearchFilter,
    initFieldSelection,
    toggleField,
    setAllFields,
    clearResults,
    subscribeToUpdates: subscribeDocumentUpdates,
    setQueryTarget,
  } = useDocumentStore();

  // Tree selection state
  const [treeSelection, setTreeSelection] = useState<TreeSelection | null>(null);
  // Field selection dialog
  const [fieldDialogOpen, setFieldDialogOpen] = useState(false);
  // Elapsed time for waiting status
  const [elapsedSeconds, setElapsedSeconds] = useState(0);

  const actionButtons = useActionButtons();

  // Load contracts and subscribe to events on mount
  useEffect(() => {
    loadContracts();
    const unsubPromises = [subscribeContractUpdates(), subscribeDocumentUpdates()];
    return () => {
      unsubPromises.forEach((p) => p.then((unsub) => unsub()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Elapsed time timer
  useEffect(() => {
    if (queryStatus !== "waiting" || !queryStartedAt) {
      setElapsedSeconds(0);
      return;
    }
    const interval = setInterval(() => {
      setElapsedSeconds(Math.floor((Date.now() - queryStartedAt) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, [queryStatus, queryStartedAt]);

  // Handle tree selection: expand contract
  const handleExpandContract = useCallback(
    (contractId: string) => {
      selectContract(contractId);
    },
    [selectContract],
  );

  // Handle tree selection: select document type
  const handleSelectDocumentType = useCallback(
    (contractId: string, documentType: string, properties: string[]) => {
      setTreeSelection({ contractId, documentType });
      setQueryTarget(contractId, documentType);
      setQueryText(`SELECT * FROM ${documentType}`);
      initFieldSelection(properties);
    },
    [setQueryTarget, setQueryText, initFieldSelection],
  );

  // Handle tree selection: select index
  const handleSelectIndex = useCallback(
    (contractId: string, documentType: string, index: IndexInfo) => {
      setTreeSelection({ contractId, documentType, indexName: index.name });
      setQueryTarget(contractId, documentType);
      // Build a WHERE clause from index properties
      const whereFields = index.properties.map((p) => p.field).join(", ");
      setQueryText(`SELECT * FROM ${documentType} WHERE ${whereFields}`);
    },
    [setQueryTarget, setQueryText],
  );

  // Clear tree selection
  const handleClearSelection = useCallback(() => {
    setTreeSelection(null);
    setQueryTarget(null, null);
    clearResults();
    setQueryText("");
  }, [setQueryTarget, clearResults, setQueryText]);

  // Handle remove contract
  const handleRemoveContract = useCallback(
    async (contractId: string) => {
      await removeContract(contractId);
      if (selectedContractId === contractId) {
        handleClearSelection();
      }
      toast.success("Contract removed");
    },
    [removeContract, selectedContractId, handleClearSelection],
  );

  // Copy contract hex
  const handleCopyHex = useCallback((contractId: string) => {
    navigator.clipboard.writeText(contractId);
    toast.success("Contract ID copied");
  }, []);

  // Copy contract JSON
  const handleCopyJson = useCallback((_contractId: string, json: string) => {
    navigator.clipboard.writeText(json);
    toast.success("Contract JSON copied");
  }, []);

  // Fetch documents
  const handleFetchDocuments = useCallback(() => {
    if (!queryContractId || !queryDocumentType) {
      toast.error("Select a contract and document type first");
      return;
    }
    fetchDocuments(queryContractId, queryDocumentType);
  }, [queryContractId, queryDocumentType, fetchDocuments]);

  // Pagination
  const handleNextPage = useCallback(() => {
    goToNextPage();
  }, [goToNextPage]);

  const handlePreviousPage = useCallback(() => {
    goToPreviousPage();
  }, [goToPreviousPage]);

  // Filter documents by search term and field selection
  const filteredDocuments = useMemo(() => {
    const results: { entry: DocumentPageEntry; formatted: string }[] = [];
    for (const entry of documents) {
      const filtered = filterDocumentFields(entry, fieldSelection);
      if (!filtered) continue;
      const formatted = formatDocument(filtered, displayMode);
      if (
        searchFilter &&
        !formatted.toLowerCase().includes(searchFilter.toLowerCase())
      ) {
        continue;
      }
      results.push({ entry, formatted });
    }
    return results;
  }, [documents, fieldSelection, displayMode, searchFilter]);

  // Navigate to action route
  const handleActionClick = useCallback(
    (route?: string) => {
      if (route) {
        navigate({ to: route });
      }
    },
    [navigate],
  );

  return (
    <div className="flex flex-1 gap-3 p-3 overflow-hidden" data-testid="document-query-screen">
      {/* Left: Contract Tree Panel */}
      <div className="w-72 shrink-0">
        <ContractTreePanel
          contracts={contracts}
          selectedContractDetail={selectedContractDetail}
          selection={treeSelection}
          loading={contractsLoading}
          onExpandContract={handleExpandContract}
          onSelectDocumentType={handleSelectDocumentType}
          onSelectIndex={handleSelectIndex}
          onClearSelection={handleClearSelection}
          onRemoveContract={handleRemoveContract}
          onCopyHex={handleCopyHex}
          onCopyJson={handleCopyJson}
          className="h-full"
        />
      </div>

      {/* Right: Query + Results */}
      <div className="flex-1 min-w-0 flex flex-col">
        <Island className="flex-1 flex flex-col overflow-hidden" noPadding>
          <div className="p-4 pb-0 space-y-3">
            {/* Page Header with Action Buttons */}
            <PageHeader
              title="Document Query"
              actions={
                <div className="flex flex-wrap gap-1.5">
                  {actionButtons.map((btn) => (
                    <Button
                      key={btn.label}
                      variant="outline"
                      size="sm"
                      className="h-7 text-xs gap-1"
                      onClick={() => handleActionClick(btn.route)}
                      data-testid={`action-${btn.label.toLowerCase().replace(/\s+/g, "-")}`}
                    >
                      {btn.icon}
                      {btn.label}
                    </Button>
                  ))}
                </div>
              }
            />

            {/* Query Input Row */}
            <div className="flex gap-2">
              <Input
                value={queryText}
                onChange={(e) => setQueryText(e.target.value)}
                placeholder="SELECT * FROM documentType"
                className="flex-1 font-mono text-sm"
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleFetchDocuments();
                }}
                data-testid="query-input"
                aria-label="Document query"
              />
              <Button
                onClick={handleFetchDocuments}
                disabled={queryStatus === "waiting" || !queryContractId || !queryDocumentType}
                className="shrink-0"
                data-testid="fetch-documents-btn"
              >
                {queryStatus === "waiting" ? (
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                ) : (
                  <Search className="h-4 w-4 mr-2" />
                )}
                Fetch Documents
              </Button>
            </div>

            {/* Controls Row — only when we have results */}
            {documents.length > 0 && (
              <div className="flex items-center gap-3 flex-wrap">
                {/* Search filter */}
                <div className="relative flex-1 min-w-[200px] max-w-sm">
                  <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                  <Input
                    value={searchFilter}
                    onChange={(e) => setSearchFilter(e.target.value)}
                    placeholder="Filter documents..."
                    className="pl-8 pr-8 h-8 text-sm"
                    data-testid="search-filter-input"
                    aria-label="Filter documents"
                  />
                  {searchFilter && (
                    <Button
                      variant="ghost"
                      size="icon"
                      className="absolute right-1 top-1/2 -translate-y-1/2 h-6 w-6"
                      onClick={() => setSearchFilter("")}
                      aria-label="Clear filter"
                    >
                      <X className="h-3 w-3" />
                    </Button>
                  )}
                </div>

                {/* Select Properties button */}
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5"
                  onClick={() => setFieldDialogOpen(true)}
                  data-testid="select-properties-btn"
                >
                  <SlidersHorizontal className="h-3.5 w-3.5" />
                  Select Properties
                </Button>

                {/* Display mode toggle */}
                <div className="flex items-center gap-1 text-sm">
                  <span className="text-muted-foreground mr-1">Display as:</span>
                  <Button
                    variant={displayMode === "yaml" ? "default" : "outline"}
                    size="sm"
                    className="h-7 text-xs px-3"
                    onClick={() => setDisplayMode("yaml")}
                    data-testid="display-mode-yaml"
                  >
                    YAML
                  </Button>
                  <Button
                    variant={displayMode === "json" ? "default" : "outline"}
                    size="sm"
                    className="h-7 text-xs px-3"
                    onClick={() => setDisplayMode("json")}
                    data-testid="display-mode-json"
                  >
                    JSON
                  </Button>
                </div>
              </div>
            )}
          </div>

          {/* Results Area */}
          <div className="flex-1 overflow-hidden px-4 py-3">
            {queryStatus === "idle" && documents.length === 0 && (
              <EmptyState
                title="Query Documents"
                description='Select a contract and document type on the left, then click "Fetch Documents" to query documents.'
                icon={Search}
              />
            )}

            {queryStatus === "waiting" && (
              <div
                className="flex items-center gap-3 justify-center py-8 text-muted-foreground"
                data-testid="query-loading"
              >
                <Loader2 className="h-5 w-5 animate-spin text-primary" />
                <span className="text-sm">
                  Fetching documents... Time taken: {elapsedSeconds}s
                </span>
              </div>
            )}

            {queryStatus === "error" && queryError && (
              <div
                className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive"
                data-testid="query-error"
              >
                {queryError}
              </div>
            )}

            {queryStatus === "complete" && documents.length === 0 && (
              <div data-testid="no-documents">
                <EmptyState
                  icon={FileQuestion}
                  title="No Documents Found"
                  description="The query returned no results. Try a different document type or query."
                />
              </div>
            )}

            {queryStatus === "complete" && filteredDocuments.length === 0 && documents.length > 0 && (
              <div data-testid="no-filtered-documents">
                <EmptyState
                  icon={Filter}
                  title="No Matches"
                  description="No documents match your filter. Try adjusting the search term."
                  actionLabel="Clear Filter"
                  onAction={() => setSearchFilter("")}
                />
              </div>
            )}

            {(queryStatus === "complete" || documents.length > 0) && filteredDocuments.length > 0 && (
              <ScrollArea className="h-full">
                <pre
                  className="text-xs font-mono whitespace-pre-wrap break-all p-3 bg-muted/30 rounded-lg border"
                  data-testid="document-results"
                >
                  {filteredDocuments.map((d) => d.formatted).join("\n\n---\n\n")}
                </pre>
              </ScrollArea>
            )}
          </div>

          {/* Pagination Controls */}
          {queryStatus === "complete" && documents.length > 0 && (
            <div
              className="flex items-center gap-3 px-4 py-3 border-t"
              data-testid="pagination-controls"
            >
              <Button
                variant="outline"
                size="sm"
                disabled={currentPage <= 1}
                onClick={handlePreviousPage}
                data-testid="previous-page-btn"
              >
                <ChevronLeft className="h-4 w-4 mr-1" />
                Previous
              </Button>
              <span className="text-sm text-muted-foreground" data-testid="page-indicator">
                Page {currentPage}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={!hasNextPage}
                onClick={handleNextPage}
                data-testid="next-page-btn"
              >
                Next
                <ChevronRight className="h-4 w-4 ml-1" />
              </Button>
            </div>
          )}
        </Island>
      </div>

      {/* Field Selection Dialog */}
      <FieldSelectionDialog
        open={fieldDialogOpen}
        onOpenChange={setFieldDialogOpen}
        fieldSelection={fieldSelection}
        onToggleField={toggleField}
        onSelectAll={() => setAllFields(true)}
        onDeselectAll={() => setAllFields(false)}
      />
    </div>
  );
}
