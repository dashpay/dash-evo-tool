import { useState, useMemo, useCallback, useRef } from "react";
import { Search, ChevronRight, ChevronDown, FileText, Database, Coins, Code, Trash2, Copy, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ConfirmationDialog } from "@/components/shared/ConfirmationDialog";
import type { ContractSummaryDto, DataContractDto, JsonValue } from "@/bindings";

// ─── Constants ────────────────────────────────────────────────────────

/** System contracts that cannot be removed by the user. */
const SYSTEM_CONTRACT_ALIASES = new Set([
  "dpns",
  "keyword_search",
  "token_history",
  "withdrawals",
  "dashpay",
]);

/** Human-friendly display names for built-in contracts. */
const DISPLAY_NAMES: Record<string, string> = {
  dpns: "DPNS",
  keyword_search: "Keyword Search",
  token_history: "Token History",
  withdrawals: "Withdrawals",
  dashpay: "DashPay",
};

// ─── Types ────────────────────────────────────────────────────────────

/** What the user has selected in the tree. */
export interface TreeSelection {
  contractId: string;
  documentType?: string;
  indexName?: string;
}

/** Index info parsed from the contract schema. */
export interface IndexInfo {
  name: string;
  properties: IndexProperty[];
  unique: boolean;
}

export interface IndexProperty {
  field: string;
  ascending: boolean;
}

/** Token info parsed from the contract schema. */
export interface TokenInfo {
  position: string;
  baseSupply?: number | string;
  maxSupply?: number | string | null;
}

/** Parsed detail for a single document type. */
export interface DocumentTypeDetail {
  name: string;
  properties: string[];
  indexes: IndexInfo[];
}

// ─── Schema parsing helpers ───────────────────────────────────────────

/** Extract document type details from contract schema JSON. */
export function parseDocumentTypes(
  schemaJson: JsonValue,
  documentTypeNames: string[],
): DocumentTypeDetail[] {
  if (!schemaJson || typeof schemaJson !== "object" || Array.isArray(schemaJson)) {
    return documentTypeNames.map((name) => ({
      name,
      properties: [],
      indexes: [],
    }));
  }

  const schema = schemaJson as Record<string, JsonValue>;

  // The schema typically has a "documentSchemas" key containing document type definitions
  const docSchemas =
    (schema.documentSchemas as Record<string, JsonValue>) ??
    (schema.document_schemas as Record<string, JsonValue>);

  return documentTypeNames.map((name) => {
    const docSchema = docSchemas?.[name];
    if (!docSchema || typeof docSchema !== "object" || Array.isArray(docSchema)) {
      return { name, properties: [], indexes: [] };
    }

    const doc = docSchema as Record<string, JsonValue>;

    // Extract properties
    const propsObj = doc.properties as Record<string, JsonValue> | undefined;
    const properties = propsObj ? Object.keys(propsObj) : [];

    // Extract indexes
    const indexesArr = doc.indices ?? doc.indexes;
    const indexes: IndexInfo[] = [];
    if (Array.isArray(indexesArr)) {
      for (const idx of indexesArr) {
        if (idx && typeof idx === "object" && !Array.isArray(idx)) {
          const idxObj = idx as Record<string, JsonValue>;
          const idxName = (idxObj.name as string) ?? "unnamed";
          const unique = (idxObj.unique as boolean) ?? false;
          const idxProps: IndexProperty[] = [];
          const propsList = idxObj.properties;
          if (Array.isArray(propsList)) {
            for (const prop of propsList) {
              if (prop && typeof prop === "object" && !Array.isArray(prop)) {
                const propObj = prop as Record<string, JsonValue>;
                const entries = Object.entries(propObj);
                const firstEntry = entries[0];
                if (firstEntry) {
                  const [field, order] = firstEntry;
                  idxProps.push({
                    field,
                    ascending: order === "asc",
                  });
                }
              }
            }
          }
          indexes.push({ name: idxName, unique, properties: idxProps });
        }
      }
    }

    return { name, properties, indexes };
  });
}

/** Extract token info from contract schema JSON. */
export function parseTokens(schemaJson: JsonValue): TokenInfo[] {
  if (!schemaJson || typeof schemaJson !== "object" || Array.isArray(schemaJson)) {
    return [];
  }

  const schema = schemaJson as Record<string, JsonValue>;
  const tokens = schema.tokens ?? schema.tokenConfiguration;
  if (!tokens || typeof tokens !== "object") return [];

  // Tokens are stored as a map with positional keys
  if (Array.isArray(tokens)) {
    return tokens
      .map((token, i) => parseTokenEntry(String(i), token))
      .filter((t): t is TokenInfo => t !== null);
  }

  const tokensMap = tokens as Record<string, JsonValue>;
  return Object.entries(tokensMap)
    .map(([pos, token]) => parseTokenEntry(pos, token))
    .filter((t): t is TokenInfo => t !== null);
}

function parseTokenEntry(position: string, token: JsonValue): TokenInfo | null {
  if (!token || typeof token !== "object" || Array.isArray(token)) return null;
  const t = token as Record<string, JsonValue>;
  const conventions = t.conventions as Record<string, JsonValue> | undefined;
  // Try to get localizedName from conventions
  const localizations = conventions?.localizations as Record<string, JsonValue> | undefined;
  // Get first localization name
  let name: string | undefined;
  if (localizations) {
    const firstLoc = Object.values(localizations)[0];
    if (firstLoc && typeof firstLoc === "object" && !Array.isArray(firstLoc)) {
      name = (firstLoc as Record<string, JsonValue>).name as string | undefined;
    }
  }

  // For supply fields, prefer camelCase but fall back to snake_case
  // Use explicit "in" check so that null values are preserved (null ?? fallback would skip null)
  const baseSupply = "baseSupply" in t ? t.baseSupply : t.base_supply;
  const maxSupply = "maxSupply" in t ? t.maxSupply : t.max_supply;

  return {
    position: name ?? `Token ${position}`,
    baseSupply: baseSupply as number | string | undefined,
    maxSupply: maxSupply as number | string | null | undefined,
  };
}

// ─── Props ────────────────────────────────────────────────────────────

export interface ContractTreePanelProps {
  /** List of contract summaries. */
  contracts: ContractSummaryDto[];
  /** Loaded contract details keyed by contract ID. */
  contractDetails: Record<string, DataContractDto>;
  /** Currently selected tree node. */
  selection: TreeSelection | null;
  /** Whether the contract list is loading. */
  loading?: boolean;
  /** Called when a contract is expanded/collapsed (to trigger detail fetch). */
  onExpandContract: (contractId: string) => void;
  /** Called when a document type is selected (updates query target). */
  onSelectDocumentType: (contractId: string, documentType: string, properties: string[]) => void;
  /** Called when an index is selected (updates query with WHERE clause). */
  onSelectIndex: (contractId: string, documentType: string, index: IndexInfo) => void;
  /** Called when selection is cleared (collapsing an expanded node). */
  onClearSelection: () => void;
  /** Called when a contract should be removed. */
  onRemoveContract: (contractId: string) => void;
  /** Called when contract hex should be copied. */
  onCopyHex?: (contractId: string) => void;
  /** Called when contract JSON should be copied. */
  onCopyJson?: (contractId: string, json: string) => void;
  /** Called when user clicks Contract JSON to view in the main area. */
  onSelectContractJson?: (contractId: string) => void;
  className?: string;
}

// ─── Tree node expansion state ────────────────────────────────────────

type ExpandedNodes = Set<string>;

function toggleNode(nodes: ExpandedNodes, key: string): ExpandedNodes {
  const next = new Set(nodes);
  if (next.has(key)) {
    next.delete(key);
  } else {
    next.add(key);
  }
  return next;
}

// ─── Component ────────────────────────────────────────────────────────

export function ContractTreePanel({
  contracts,
  contractDetails,
  selection,
  loading = false,
  onExpandContract,
  onSelectDocumentType,
  onSelectIndex,
  onClearSelection,
  onRemoveContract,
  onCopyHex,
  onCopyJson,
  onSelectContractJson,
  className,
}: ContractTreePanelProps) {
  const [searchFilter, setSearchFilter] = useState("");
  const [expandedNodes, setExpandedNodes] = useState<ExpandedNodes>(new Set());
  const [removeTarget, setRemoveTarget] = useState<{
    contractId: string;
    name: string;
  } | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Filter contracts by search term
  const filteredContracts = useMemo(() => {
    if (!searchFilter.trim()) return contracts;
    const term = searchFilter.toLowerCase();
    return contracts.filter((c) => {
      const displayName = getDisplayName(c);
      return (
        displayName.toLowerCase().includes(term) ||
        c.id.toLowerCase().includes(term)
      );
    });
  }, [contracts, searchFilter]);

  const handleToggleContract = useCallback(
    (contractId: string) => {
      const key = `contract:${contractId}`;
      const isExpanding = !expandedNodes.has(key);
      setExpandedNodes((prev) => toggleNode(prev, key));
      if (isExpanding) {
        onExpandContract(contractId);
      } else {
        // If collapsing the currently selected contract, clear selection
        if (selection?.contractId === contractId) {
          onClearSelection();
        }
      }
    },
    [expandedNodes, onExpandContract, onClearSelection, selection],
  );

  const handleToggleSection = useCallback(
    (key: string) => {
      setExpandedNodes((prev) => toggleNode(prev, key));
    },
    [],
  );

  const handleSelectDocType = useCallback(
    (contractId: string, dt: DocumentTypeDetail) => {
      const key = `doctype:${contractId}:${dt.name}`;
      const isExpanding = !expandedNodes.has(key);
      setExpandedNodes((prev) => toggleNode(prev, key));

      if (isExpanding) {
        onSelectDocumentType(contractId, dt.name, dt.properties);
      } else {
        // Collapsing doc type: clear index selection, update query
        if (
          selection?.contractId === contractId &&
          selection?.documentType === dt.name
        ) {
          onSelectDocumentType(contractId, dt.name, dt.properties);
        }
      }
    },
    [expandedNodes, onSelectDocumentType, selection],
  );

  const handleSelectIndex = useCallback(
    (contractId: string, docTypeName: string, index: IndexInfo) => {
      const key = `index:${contractId}:${docTypeName}:${index.name}`;
      const isExpanding = !expandedNodes.has(key);
      setExpandedNodes((prev) => toggleNode(prev, key));

      if (isExpanding) {
        onSelectIndex(contractId, docTypeName, index);
      } else {
        // Collapsing index: find the parent doc type and re-select it
        const detail = contractDetails[contractId];
        if (detail) {
          const docTypes = parseDocumentTypes(detail.schemaJson, detail.documentTypeNames);
          const dt = docTypes.find((d) => d.name === docTypeName);
          if (dt) {
            onSelectDocumentType(contractId, docTypeName, dt.properties);
          }
        }
      }
    },
    [expandedNodes, onSelectIndex, contractDetails, onSelectDocumentType],
  );

  const handleClearSearch = useCallback(() => {
    setSearchFilter("");
    searchInputRef.current?.focus();
  }, []);

  const handleRemoveConfirm = useCallback(
    (status: "confirmed" | "canceled") => {
      if (status === "confirmed" && removeTarget) {
        onRemoveContract(removeTarget.contractId);
      }
      setRemoveTarget(null);
    },
    [removeTarget, onRemoveContract],
  );

  const isSystemContract = useCallback((contract: ContractSummaryDto) => {
    return contract.alias !== null && SYSTEM_CONTRACT_ALIASES.has(contract.alias);
  }, []);

  // ─── Render ───────────────────────────────────────────────────────

  return (
    <div
      className={cn(
        "flex flex-col h-full bg-card border rounded-lg shadow-sm",
        className,
      )}
      data-testid="contract-tree-panel"
    >
      {/* Search bar */}
      <div className="p-3 border-b">
        <div className="relative">
          <Search
            className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            ref={searchInputRef}
            placeholder="Filter contracts..."
            value={searchFilter}
            onChange={(e) => setSearchFilter(e.target.value)}
            className="pl-8 pr-8 h-8 text-sm"
            data-testid="contract-search-input"
            aria-label="Filter contracts"
          />
          {searchFilter && (
            <Button
              variant="ghost"
              size="icon"
              className="absolute right-1 top-1/2 -translate-y-1/2 h-6 w-6"
              onClick={handleClearSearch}
              aria-label="Clear search"
            >
              <X className="h-3 w-3" />
            </Button>
          )}
        </div>
      </div>

      {/* Tree content */}
      <ScrollArea className="flex-1">
        <div className="p-2" role={filteredContracts.length > 0 ? "tree" : undefined} aria-label={filteredContracts.length > 0 ? "Contract browser" : undefined}>
          {loading ? (
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              Loading contracts...
            </div>
          ) : filteredContracts.length === 0 ? (
            <div
              className="flex flex-col items-center justify-center py-8 px-4 text-center"
              data-testid="contract-tree-empty"
            >
              <Database className="h-8 w-8 text-muted-foreground/50 mb-2" aria-hidden="true" />
              <p className="text-sm text-muted-foreground">
                {contracts.length === 0
                  ? "No contracts loaded"
                  : "No contracts match your filter"}
              </p>
            </div>
          ) : (
            filteredContracts.map((contract) =>
              renderContract(
                contract,
                expandedNodes,
                contractDetails,
                selection,
                handleToggleContract,
                handleToggleSection,
                handleSelectDocType,
                handleSelectIndex,
                isSystemContract,
                setRemoveTarget,
                onCopyHex,
                onCopyJson,
                onSelectContractJson,
              ),
            )
          )}
        </div>
      </ScrollArea>

      {/* Remove confirmation dialog */}
      <ConfirmationDialog
        open={removeTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRemoveTarget(null);
        }}
        title="Remove Contract"
        message={`Are you sure you want to remove "${removeTarget?.name}"? This will remove it from your local database.`}
        confirmText="Remove"
        danger
        onResult={handleRemoveConfirm}
      />
    </div>
  );
}

// ─── Sub-render functions ─────────────────────────────────────────────

function getDisplayName(contract: ContractSummaryDto): string {
  if (contract.alias) {
    return DISPLAY_NAMES[contract.alias] ?? contract.alias;
  }
  // Show truncated ID
  return contract.id.length > 12
    ? `${contract.id.slice(0, 6)}...${contract.id.slice(-6)}`
    : contract.id;
}

function renderContract(
  contract: ContractSummaryDto,
  expandedNodes: ExpandedNodes,
  contractDetails: Record<string, DataContractDto>,
  selection: TreeSelection | null,
  onToggleContract: (id: string) => void,
  onToggleSection: (key: string) => void,
  onSelectDocType: (contractId: string, dt: DocumentTypeDetail) => void,
  onSelectIndex: (contractId: string, docType: string, index: IndexInfo) => void,
  isSystemContract: (contract: ContractSummaryDto) => boolean,
  setRemoveTarget: (target: { contractId: string; name: string } | null) => void,
  onCopyHex?: (contractId: string) => void,
  onCopyJson?: (contractId: string, json: string) => void,
  onSelectContractJson?: (contractId: string) => void,
) {
  const contractKey = `contract:${contract.id}`;
  const isExpanded = expandedNodes.has(contractKey);
  const isSelected = selection?.contractId === contract.id;
  const displayName = getDisplayName(contract);
  const detail = contractDetails[contract.id] ?? null;
  const isDetail = detail !== null;

  // Parse doc types and tokens for this specific contract
  const documentTypes = isDetail
    ? parseDocumentTypes(detail.schemaJson, detail.documentTypeNames)
    : [];
  const tokens = isDetail ? parseTokens(detail.schemaJson) : [];

  return (
    <div key={contract.id} role="treeitem" aria-expanded={isExpanded} data-testid={`contract-node-${contract.id}`}>
      {/* Contract header row */}
      <div className="flex items-center group">
        <button
          className={cn(
            "flex items-center gap-1.5 flex-1 min-w-0 rounded-md px-2 py-1.5 text-left text-sm font-semibold transition-colors",
            "hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            isSelected && "text-primary",
          )}
          onClick={() => onToggleContract(contract.id)}
          aria-label={`${isExpanded ? "Collapse" : "Expand"} ${displayName}`}
          data-testid={`contract-toggle-${contract.id}`}
        >
          {isExpanded ? (
            <ChevronDown className="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
          ) : (
            <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="truncate">{displayName}</span>
            </TooltipTrigger>
            <TooltipContent side="right">
              <p className="font-mono text-xs">{contract.id}</p>
              {contract.alias && (
                <p className="text-xs text-muted-foreground">Alias: {contract.alias}</p>
              )}
              <p className="text-xs text-muted-foreground">
                {contract.documentTypeCount} doc type{contract.documentTypeCount !== 1 ? "s" : ""}
                {contract.tokenCount > 0 && `, ${contract.tokenCount} token${contract.tokenCount !== 1 ? "s" : ""}`}
              </p>
            </TooltipContent>
          </Tooltip>
        </button>

        {/* Context menu (copy actions) */}
        {isExpanded && isDetail && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
                aria-label="Contract actions"
                data-testid={`contract-menu-${contract.id}`}
              >
                <Copy className="h-3.5 w-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" side="right">
              {onCopyHex && (
                <DropdownMenuItem
                  onClick={() => onCopyHex(contract.id)}
                  data-testid="copy-hex-action"
                >
                  <Copy className="h-4 w-4 mr-2" aria-hidden="true" />
                  Copy (Hex)
                </DropdownMenuItem>
              )}
              {onCopyJson && detail && (
                <DropdownMenuItem
                  onClick={() => {
                    const json = JSON.stringify(detail.schemaJson, null, 2);
                    onCopyJson(contract.id, json);
                  }}
                  data-testid="copy-json-action"
                >
                  <Code className="h-4 w-4 mr-2" aria-hidden="true" />
                  Copy (JSON)
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>

      {/* Expanded content */}
      {isExpanded && (
        <div className="ml-3 border-l border-border/50 pl-1">
          {isDetail ? (
            <>
              {/* Document Types section */}
              {documentTypes.length > 0 &&
                renderDocumentTypesSection(
                  contract.id,
                  expandedNodes,
                  documentTypes,
                  selection,
                  onToggleSection,
                  onSelectDocType,
                  onSelectIndex,
                )}

              {/* Tokens section */}
              {tokens.length > 0 &&
                renderTokensSection(
                  contract.id,
                  expandedNodes,
                  tokens,
                  onToggleSection,
                )}

              {/* Contract JSON — click to view in main area */}
              {renderContractJsonSection(
                contract.id,
                onSelectContractJson,
              )}

              {/* Remove button for non-system contracts */}
              {!isSystemContract(contract) && (
                <div className="mt-1 px-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="w-full justify-start text-destructive hover:text-destructive hover:bg-destructive/10 h-7 text-xs"
                    onClick={() =>
                      setRemoveTarget({
                        contractId: contract.id,
                        name: displayName,
                      })
                    }
                    data-testid={`contract-remove-${contract.id}`}
                  >
                    <Trash2 className="h-3.5 w-3.5 mr-1.5" aria-hidden="true" />
                    Remove
                  </Button>
                </div>
              )}
            </>
          ) : (
            <div className="flex items-center justify-center py-3 text-xs text-muted-foreground">
              Loading...
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function renderDocumentTypesSection(
  contractId: string,
  expandedNodes: ExpandedNodes,
  documentTypes: DocumentTypeDetail[],
  selection: TreeSelection | null,
  onToggleSection: (key: string) => void,
  onSelectDocType: (contractId: string, dt: DocumentTypeDetail) => void,
  onSelectIndex: (contractId: string, docType: string, index: IndexInfo) => void,
) {
  const sectionKey = `section:${contractId}:docTypes`;
  const isExpanded = expandedNodes.has(sectionKey);

  return (
    <div key="doc-types" data-testid="doc-types-section">
      <TreeNodeButton
        icon={<FileText className="h-3.5 w-3.5" aria-hidden="true" />}
        label="Document Types"
        isExpanded={isExpanded}
        level={1}
        onClick={() => onToggleSection(sectionKey)}
        testId="doc-types-toggle"
      />

      {isExpanded &&
        documentTypes.map((dt) => {
          const dtKey = `doctype:${contractId}:${dt.name}`;
          const isDtExpanded = expandedNodes.has(dtKey);
          const isSelected =
            selection?.contractId === contractId &&
            selection?.documentType === dt.name &&
            !selection?.indexName;

          return (
            <div key={dt.name} data-testid={`doctype-node-${dt.name}`}>
              <TreeNodeButton
                label={dt.name}
                isExpanded={isDtExpanded}
                isSelected={isSelected}
                level={2}
                onClick={() => onSelectDocType(contractId, dt)}
                testId={`doctype-toggle-${dt.name}`}
              />

              {isDtExpanded && (
                <div className="ml-2">
                  {dt.indexes.length === 0 ? (
                    <div className="pl-6 py-1 text-xs text-muted-foreground">
                      No indexes defined
                    </div>
                  ) : (
                    dt.indexes.map((idx) =>
                      renderIndex(
                        contractId,
                        dt.name,
                        idx,
                        expandedNodes,
                        selection,
                        onSelectIndex,
                      ),
                    )
                  )}
                </div>
              )}
            </div>
          );
        })}
    </div>
  );
}

function renderIndex(
  contractId: string,
  docTypeName: string,
  index: IndexInfo,
  expandedNodes: ExpandedNodes,
  selection: TreeSelection | null,
  onSelectIndex: (contractId: string, docType: string, index: IndexInfo) => void,
) {
  const indexKey = `index:${contractId}:${docTypeName}:${index.name}`;
  const isExpanded = expandedNodes.has(indexKey);
  const isSelected =
    selection?.contractId === contractId &&
    selection?.documentType === docTypeName &&
    selection?.indexName === index.name;

  return (
    <div key={index.name} data-testid={`index-node-${index.name}`}>
      <TreeNodeButton
        label={`Index: ${index.name}`}
        isExpanded={isExpanded}
        isSelected={isSelected}
        level={3}
        badge={index.unique ? "unique" : undefined}
        onClick={() => onSelectIndex(contractId, docTypeName, index)}
        testId={`index-toggle-${index.name}`}
      />

      {isExpanded && (
        <div className="ml-4 pl-4 py-1 space-y-0.5">
          {index.properties.map((prop) => (
            <div
              key={prop.field}
              className="text-xs text-muted-foreground font-mono"
              data-testid={`index-property-${prop.field}`}
            >
              {prop.field} ({prop.ascending ? "asc" : "desc"})
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function renderTokensSection(
  contractId: string,
  expandedNodes: ExpandedNodes,
  tokens: TokenInfo[],
  onToggleSection: (key: string) => void,
) {
  const sectionKey = `section:${contractId}:tokens`;
  const isExpanded = expandedNodes.has(sectionKey);

  return (
    <div key="tokens" data-testid="tokens-section">
      <TreeNodeButton
        icon={<Coins className="h-3.5 w-3.5" aria-hidden="true" />}
        label="Tokens"
        isExpanded={isExpanded}
        level={1}
        onClick={() => onToggleSection(sectionKey)}
        testId="tokens-toggle"
      />

      {isExpanded &&
        tokens.map((token, i) => {
          const tokenKey = `token:${contractId}:${i}`;
          const isTokenExpanded = expandedNodes.has(tokenKey);

          return (
            <div key={i} data-testid={`token-node-${i}`}>
              <TreeNodeButton
                label={token.position}
                isExpanded={isTokenExpanded}
                level={2}
                onClick={() => onToggleSection(tokenKey)}
                testId={`token-toggle-${i}`}
              />

              {isTokenExpanded && (
                <div className="ml-4 pl-4 py-1 space-y-0.5 text-xs text-muted-foreground">
                  <div data-testid={`token-base-supply-${i}`}>
                    Base Supply: {token.baseSupply ?? "N/A"}
                  </div>
                  <div data-testid={`token-max-supply-${i}`}>
                    Max Supply: {token.maxSupply ?? "None"}
                  </div>
                </div>
              )}
            </div>
          );
        })}
    </div>
  );
}

function renderContractJsonSection(
  contractId: string,
  onSelectContractJson?: (contractId: string) => void,
) {
  return (
    <div key="json" data-testid="contract-json-section">
      <button
        className={cn(
          "flex items-center gap-1.5 w-full rounded-md px-2 py-1 text-left transition-colors",
          "hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          "text-sm font-medium",
        )}
        style={{ paddingLeft: "8px" }}
        onClick={() => onSelectContractJson?.(contractId)}
        data-testid="contract-json-toggle"
      >
        <Code className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="truncate">View Contract JSON</span>
      </button>
    </div>
  );
}

// ─── Tree node button ─────────────────────────────────────────────────

interface TreeNodeButtonProps {
  icon?: React.ReactNode;
  label: string;
  isExpanded: boolean;
  isSelected?: boolean;
  level: number;
  badge?: string;
  onClick: () => void;
  testId?: string;
}

function TreeNodeButton({
  icon,
  label,
  isExpanded,
  isSelected = false,
  level,
  badge,
  onClick,
  testId,
}: TreeNodeButtonProps) {
  const paddingLeft = level * 8;

  return (
    <button
      className={cn(
        "flex items-center gap-1.5 w-full rounded-md px-2 py-1 text-left transition-colors",
        "hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        isSelected && "bg-accent/70 text-primary font-medium",
        level === 1 && "text-sm font-medium",
        level === 2 && "text-[13px]",
        level >= 3 && "text-xs",
      )}
      style={{ paddingLeft: `${paddingLeft}px` }}
      onClick={onClick}
      aria-label={`${isExpanded ? "Collapse" : "Expand"} ${label}`}
      data-testid={testId}
    >
      {isExpanded ? (
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-primary" aria-hidden="true" />
      ) : (
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      )}
      {icon}
      <span className="truncate">{label}</span>
      {badge && (
        <span className="ml-auto text-[10px] text-muted-foreground bg-muted px-1.5 py-0.5 rounded-full shrink-0">
          {badge}
        </span>
      )}
    </button>
  );
}
