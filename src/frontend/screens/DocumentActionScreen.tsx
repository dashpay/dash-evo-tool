import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronUp,
  Eye,
  Loader2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Island, PageHeader } from "@/components/layout";
import { IdentitySelector } from "@/components/shared/IdentitySelector";
import {
  WalletUnlockDialog,
  type WalletUnlockResult,
} from "@/components/shared/WalletUnlockDialog";
import { commands, events } from "@/bindings";
import type {
  TaskResultEvent,
  TaskErrorEvent,
  QualifiedIdentityDto,
  DataContractDto,
  JsonValue,
  TokenPaymentInfoDto,
} from "@/bindings";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { useWalletStore } from "@/stores/walletStore";
import type { WalletDto, SingleKeyWalletDto } from "@/bindings";
import { Checkbox } from "@/components/ui/checkbox";

// ─── Types ────────────────────────────────────────────────────────────

export type DocumentActionType =
  | "create"
  | "delete"
  | "replace"
  | "transfer"
  | "purchase"
  | "setPrice";

type ScreenStatus =
  | { type: "input" }
  | { type: "fetching"; startTime: number }
  | { type: "fetched" }
  | { type: "broadcasting"; startTime: number }
  | { type: "success" }
  | { type: "error"; message: string };

interface WalletInfo {
  seedHash: string;
  alias: string | null;
  usesPassword: boolean;
  passwordHint: string | null;
}

/** Token cost information derived from document type schema. */
interface TokenCostInfo {
  tokenAmount: number;
  tokenName: string;
  effect: string;
  gasFeesPaidBy: string;
  tokenId: string;
}

/** Parsed document property schema. */
interface PropertySchema {
  type: string;
  required: boolean;
  maxLength?: number;
  minLength?: number;
  description?: string;
}

// ─── Constants ────────────────────────────────────────────────────────

const CREDITS_PER_DASH = 100_000_000_000;

const ACTION_LABELS: Record<DocumentActionType, string> = {
  create: "Create Document",
  delete: "Delete Document",
  replace: "Replace Document",
  transfer: "Transfer Document",
  purchase: "Purchase Document",
  setPrice: "Set Document Price",
};

const BROADCAST_LABELS: Record<DocumentActionType, string> = {
  create: "Broadcast document",
  delete: "Delete document",
  replace: "Replace document",
  transfer: "Transfer document",
  purchase: "Purchase document",
  setPrice: "Set document price",
};

// ─── Helpers ──────────────────────────────────────────────────────────

function formatCreditsAsDash(credits: number): string {
  return (credits / CREDITS_PER_DASH).toFixed(8);
}

function formatElapsed(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds} second${seconds === 1 ? "" : "s"}`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes} minute${minutes === 1 ? "" : "s"} and ${remainingSeconds} second${remainingSeconds === 1 ? "" : "s"}`;
}

/** Auto-select first suitable AUTHENTICATION key (HIGH or CRITICAL, not MASTER). */
function autoSelectKey(identity: QualifiedIdentityDto | null): number | null {
  if (!identity) return null;
  const authKeys = identity.keys.filter(
    (k) =>
      !k.isDisabled &&
      k.purpose === "AUTHENTICATION" &&
      k.securityLevel !== "MASTER",
  );
  const highKey = authKeys.find((k) => k.securityLevel === "HIGH");
  if (highKey) return highKey.keyId;
  const criticalKey = authKeys.find((k) => k.securityLevel === "CRITICAL");
  if (criticalKey) return criticalKey.keyId;
  const mediumKey = authKeys.find((k) => k.securityLevel === "MEDIUM");
  if (mediumKey) return mediumKey.keyId;
  return authKeys.length > 0 ? authKeys[0]!.keyId : null;
}

/** Find the wallet associated with the selected identity. */
function findAssociatedWallet(
  identity: QualifiedIdentityDto | null,
  hdWallets: WalletDto[],
  singleKeyWallets: SingleKeyWalletDto[],
): WalletInfo | null {
  if (!identity) return null;
  const hashes = identity.associatedWalletHashes;
  for (const hash of hashes) {
    const hd = hdWallets.find((w) => w.seedHash === hash);
    if (hd)
      return {
        seedHash: hd.seedHash,
        alias: hd.alias,
        usesPassword: hd.usesPassword,
        passwordHint: hd.passwordHint,
      };
  }
  for (const hash of hashes) {
    const sk = singleKeyWallets.find((w) => w.keyHash === hash);
    if (sk)
      return {
        seedHash: sk.keyHash,
        alias: sk.alias,
        usesPassword: sk.usesPassword,
        passwordHint: null,
      };
  }
  return null;
}

/**
 * Parse document type properties from the contract schema JSON.
 * Returns a map of property name → PropertySchema.
 */
function parseDocumentTypeProperties(
  contract: DataContractDto | null,
  documentTypeName: string | null,
): Record<string, PropertySchema> {
  if (!contract || !documentTypeName) return {};
  const schema = contract.schemaJson as Record<string, JsonValue>;
  if (!schema) return {};

  const docSchemas =
    (schema.documentSchemas as Record<string, JsonValue>) ??
    (schema.document_schemas as Record<string, JsonValue>);
  if (!docSchemas) return {};

  const docSchema = docSchemas[documentTypeName];
  if (!docSchema || typeof docSchema !== "object" || Array.isArray(docSchema))
    return {};

  const doc = docSchema as Record<string, JsonValue>;
  const propsObj = doc.properties as Record<string, JsonValue> | undefined;
  if (!propsObj) return {};

  const requiredFields = Array.isArray(doc.required)
    ? new Set(doc.required as string[])
    : new Set<string>();

  const result: Record<string, PropertySchema> = {};
  for (const [name, propDef] of Object.entries(propsObj)) {
    if (!propDef || typeof propDef !== "object" || Array.isArray(propDef))
      continue;
    const prop = propDef as Record<string, JsonValue>;
    const propType = (prop.type as string) ?? "string";

    // Skip system/internal fields
    if (name.startsWith("$")) continue;

    result[name] = {
      type: propType,
      required: requiredFields.has(name),
      maxLength: prop.maxLength as number | undefined,
      minLength: prop.minLength as number | undefined,
      description: prop.description as string | undefined,
    };

    // Handle byte arrays specifically
    if (prop.contentMediaType || prop.byteArray) {
      result[name].type = "byteArray";
    }

    // Handle identifiers
    if (
      propType === "array" &&
      prop.byteArray === true &&
      (prop.minItems === 32 || prop.contentMediaType === "application/x.dash.dpp.identifier")
    ) {
      result[name].type = "identifier";
    }

    // Handle dates
    if (prop.format === "date-time" || prop.description?.toString().toLowerCase().includes("timestamp")) {
      result[name].type = "date";
    }
  }

  return result;
}

/**
 * Check if a document type has an $ownerId index (for fetch-owned-documents feature).
 */
function hasOwnerIdIndex(
  contract: DataContractDto | null,
  documentTypeName: string | null,
): boolean {
  if (!contract || !documentTypeName) return false;
  const schema = contract.schemaJson as Record<string, JsonValue>;
  if (!schema) return false;

  const docSchemas =
    (schema.documentSchemas as Record<string, JsonValue>) ??
    (schema.document_schemas as Record<string, JsonValue>);
  if (!docSchemas) return false;

  const docSchema = docSchemas[documentTypeName];
  if (!docSchema || typeof docSchema !== "object" || Array.isArray(docSchema))
    return false;

  const doc = docSchema as Record<string, JsonValue>;
  const indexesArr = doc.indices ?? doc.indexes;
  if (!Array.isArray(indexesArr)) return false;

  for (const idx of indexesArr) {
    if (idx && typeof idx === "object" && !Array.isArray(idx)) {
      const idxObj = idx as Record<string, JsonValue>;
      const propsList = idxObj.properties;
      if (Array.isArray(propsList)) {
        for (const prop of propsList) {
          if (prop && typeof prop === "object" && !Array.isArray(prop)) {
            const keys = Object.keys(prop as Record<string, unknown>);
            if (keys.includes("$ownerId")) return true;
          }
        }
      }
    }
  }
  return false;
}

/**
 * Parse token cost info from a document type's schema.
 */
function parseTokenCost(
  _contract: DataContractDto | null,
  _documentTypeName: string | null,
  _actionType: DocumentActionType,
): TokenCostInfo | null {
  // Token costs are embedded in the document schema under specific keys
  // For now, return null — actual token cost parsing would need backend support
  // to extract from the document type definition
  return null;
}

/**
 * Estimate fee for a document action (simple client-side estimate).
 */
function estimateFee(actionType: DocumentActionType, dataSize: number): number {
  const baseFee = 20 * 5000; // 100,000 credits for tree ops
  switch (actionType) {
    case "create":
      return baseFee + dataSize * 50;
    case "delete":
      return baseFee;
    case "replace":
      return baseFee + dataSize * 50;
    case "transfer":
      return baseFee + 500;
    case "purchase":
      return baseFee + 500;
    case "setPrice":
      return baseFee + 500;
    default:
      return baseFee;
  }
}

// ─── Component ────────────────────────────────────────────────────────

interface DocumentActionScreenProps {
  actionType: DocumentActionType;
}

export function DocumentActionScreen({ actionType }: DocumentActionScreenProps) {
  const navigate = useNavigate();

  // ─── Stores ──────────────────────────────────────────────────────
  const identities = useIdentityStore((s) => s.identities);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);

  const contracts = useContractStore((s) => s.contracts);
  const loadContracts = useContractStore((s) => s.loadContracts);

  const hdWallets = useWalletStore((s) => s.hdWallets);
  const singleKeyWallets = useWalletStore((s) => s.singleKeyWallets);

  // ─── Local state ─────────────────────────────────────────────────
  const [status, setStatus] = useState<ScreenStatus>({ type: "input" });
  const [elapsedMs, setElapsedMs] = useState(0);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Contract & doc type selection
  const [selectedContractId, setSelectedContractId] = useState<string>("");
  const [selectedContractDetail, setSelectedContractDetail] =
    useState<DataContractDto | null>(null);
  const [selectedDocTypeName, setSelectedDocTypeName] = useState<string>("");
  const [contractSearch] = useState("");

  // Identity & key selection
  const [rawIdentityId, setRawIdentityId] = useState<string>("");
  const [selectedKeyId, setSelectedKeyId] = useState<number | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Wallet unlock
  const [walletUnlockOpen, setWalletUnlockOpen] = useState(false);
  const [walletUnlockError, setWalletUnlockError] = useState<string | null>(
    null,
  );
  const [walletUnlockedHashes, setWalletUnlockedHashes] = useState<
    Set<string>
  >(new Set());

  // Dynamic form fields (for create & replace)
  const [fieldInputs, setFieldInputs] = useState<Record<string, string>>({});

  // Action-specific fields
  const [documentIdInput, setDocumentIdInput] = useState("");
  const [recipientIdInput, setRecipientIdInput] = useState("");
  const [priceInput, setPriceInput] = useState("");
  const [fetchedPrice, setFetchedPrice] = useState<number | null>(null);
  const [fetchedDocuments, setFetchedDocuments] = useState<
    { id: string; json: string }[]
  >([]);
  const [viewingDocumentJson, setViewingDocumentJson] = useState<string | null>(
    null,
  );
  const [originalDocument, setOriginalDocument] = useState<JsonValue | null>(
    null,
  );

  // Task tracking
  const [currentTaskId, setCurrentTaskId] = useState<string | null>(null);
  const [backendMessage, setBackendMessage] = useState<string | null>(null);
  const [feeResult, setFeeResult] = useState<string | null>(null);

  // ─── Derived values ──────────────────────────────────────────────

  // Effective identity: use rawIdentityId if valid, otherwise auto-select first
  const selectedIdentityId = useMemo(() => {
    if (rawIdentityId && identities.some((i) => i.id === rawIdentityId)) {
      return rawIdentityId;
    }
    return identities.length > 0 ? identities[0]!.id : "";
  }, [rawIdentityId, identities]);

  const selectedIdentity = useMemo(
    () => identities.find((i) => i.id === selectedIdentityId) ?? null,
    [identities, selectedIdentityId],
  );

  // Effective key: use selectedKeyId if valid on current identity, otherwise auto-select
  const effectiveKeyId = useMemo(() => {
    if (
      selectedKeyId !== null &&
      selectedIdentity?.keys.some(
        (k) => k.keyId === selectedKeyId && !k.isDisabled && k.purpose === "AUTHENTICATION" && k.securityLevel !== "MASTER",
      )
    ) {
      return selectedKeyId;
    }
    return autoSelectKey(selectedIdentity);
  }, [selectedIdentity, selectedKeyId]);

  const identityOptions = useMemo(
    () =>
      identities.map((i) => ({
        id: i.id,
        displayName: i.alias || i.id.slice(0, 16) + "...",
      })),
    [identities],
  );

  const associatedWallet = useMemo(
    () => findAssociatedWallet(selectedIdentity, hdWallets, singleKeyWallets),
    [selectedIdentity, hdWallets, singleKeyWallets],
  );

  const walletLocked = useMemo(
    () =>
      !!associatedWallet &&
      associatedWallet.usesPassword &&
      !walletUnlockedHashes.has(associatedWallet.seedHash),
    [associatedWallet, walletUnlockedHashes],
  );

  const filteredContracts = useMemo(() => {
    if (!contractSearch) return contracts;
    const search = contractSearch.toLowerCase();
    return contracts.filter(
      (c) =>
        c.id.toLowerCase().includes(search) ||
        (c.alias && c.alias.toLowerCase().includes(search)),
    );
  }, [contracts, contractSearch]);

  const documentTypeNames = useMemo(
    () => selectedContractDetail?.documentTypeNames ?? [],
    [selectedContractDetail],
  );

  const properties = useMemo(
    () =>
      parseDocumentTypeProperties(selectedContractDetail, selectedDocTypeName),
    [selectedContractDetail, selectedDocTypeName],
  );

  const canFetchOwned = useMemo(
    () => hasOwnerIdIndex(selectedContractDetail, selectedDocTypeName),
    [selectedContractDetail, selectedDocTypeName],
  );

  const tokenCost = useMemo(
    () => parseTokenCost(selectedContractDetail, selectedDocTypeName, actionType),
    [selectedContractDetail, selectedDocTypeName, actionType],
  );

  const estimatedFee = useMemo(() => {
    if (!selectedContractId || !selectedDocTypeName) return null;
    const dataSize =
      actionType === "create" || actionType === "replace"
        ? Object.values(fieldInputs).reduce(
            (acc, v) => acc + (v?.length ?? 0),
            0,
          )
        : 0;
    return estimateFee(actionType, dataSize);
  }, [actionType, selectedContractId, selectedDocTypeName, fieldInputs]);

  const canBroadcast = useMemo(() => {
    if (!selectedContractId || !selectedDocTypeName || !selectedIdentityId || effectiveKeyId === null)
      return false;
    if (walletLocked) return false;
    if (status.type !== "input" && status.type !== "fetched") return false;

    switch (actionType) {
      case "create":
        return Object.keys(fieldInputs).length > 0;
      case "delete":
        return documentIdInput.trim().length > 0;
      case "replace":
        return originalDocument !== null;
      case "transfer":
        return (
          documentIdInput.trim().length > 0 &&
          recipientIdInput.trim().length > 0
        );
      case "purchase":
        return documentIdInput.trim().length > 0 && fetchedPrice !== null;
      case "setPrice":
        return (
          documentIdInput.trim().length > 0 && priceInput.trim().length > 0
        );
      default:
        return false;
    }
  }, [
    selectedContractId,
    selectedDocTypeName,
    selectedIdentityId,
    effectiveKeyId,
    walletLocked,
    status,
    actionType,
    fieldInputs,
    documentIdInput,
    recipientIdInput,
    fetchedPrice,
    priceInput,
    originalDocument,
  ]);

  // ─── Effects ─────────────────────────────────────────────────────

  // Load identities and contracts on mount
  useEffect(() => {
    loadIdentities();
    loadContracts();
  }, [loadIdentities, loadContracts]);


  // (Contract detail loading and field reset handled in event handlers below)

  // Elapsed time tracker
  useEffect(() => {
    if (status.type === "broadcasting" || status.type === "fetching") {
      const startTime = status.startTime;
      elapsedTimerRef.current = setInterval(() => {
        setElapsedMs(Date.now() - startTime);
      }, 200);
    } else {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
        elapsedTimerRef.current = null;
      }
    }
    return () => {
      if (elapsedTimerRef.current) {
        clearInterval(elapsedTimerRef.current);
      }
    };
  }, [status]);

  // ─── Handlers ────────────────────────────────────────────────────

  const handleFetchResult = useCallback(
    (payload: JsonValue | undefined) => {
      if (!payload) {
        setStatus({ type: "error", message: "No documents returned." });
        return;
      }

      const data = payload as Record<string, unknown>;
      const documents = data.documents as JsonValue[] | undefined;

      if (actionType === "purchase") {
        // Extract price from first document
        if (documents && documents.length > 0) {
          const doc = documents[0] as Record<string, unknown>;
          const price = doc?.$price ?? doc?.price;
          if (price !== undefined && price !== null) {
            setFetchedPrice(Number(price));
          } else {
            setFetchedPrice(0);
          }
          setStatus({ type: "fetched" });
        } else {
          setStatus({ type: "error", message: "Document not found." });
        }
      } else if (actionType === "replace") {
        // Populate form with original document
        if (documents && documents.length > 0) {
          const doc = documents[0] as Record<string, unknown>;
          setOriginalDocument(doc as JsonValue);
          // Populate form fields from document properties
          const newInputs: Record<string, string> = {};
          for (const [key, value] of Object.entries(doc)) {
            if (key.startsWith("$")) continue;
            if (value === null || value === undefined) continue;
            if (typeof value === "object" && !Array.isArray(value)) {
              newInputs[key] = JSON.stringify(value);
            } else if (Array.isArray(value)) {
              // Check if it's a byte array (array of numbers)
              if (value.every((v) => typeof v === "number")) {
                newInputs[key] = Array.from(value as number[])
                  .map((b) => b.toString(16).padStart(2, "0"))
                  .join("");
              } else {
                newInputs[key] = JSON.stringify(value);
              }
            } else if (typeof value === "boolean") {
              newInputs[key] = value ? "true" : "false";
            } else {
              newInputs[key] = String(value);
            }
          }
          setFieldInputs(newInputs);
          setStatus({ type: "fetched" });
        } else {
          setStatus({ type: "error", message: "No document found with the provided ID." });
        }
      } else if (actionType === "delete") {
        // Store fetched documents for selection
        if (documents && documents.length > 0) {
          const docs = documents.map((d) => {
            const doc = d as Record<string, unknown>;
            const id = doc.$id ?? doc.id ?? "";
            return {
              id: String(id),
              json: JSON.stringify(d, null, 2),
            };
          });
          setFetchedDocuments(docs);
          setStatus({ type: "fetched" });
        } else {
          setFetchedDocuments([]);
          setStatus({ type: "fetched" });
        }
      }
    },
    [actionType],
  );

  // Subscribe to task events
  useEffect(() => {
    let unlistenResult: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;

    (async () => {
      unlistenResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const { taskId, resultType, payload } = event.payload;
          if (taskId !== currentTaskId || resultType !== "Document") return;

          if (status.type === "fetching") {
            // Handle fetch results
            handleFetchResult(payload);
          } else {
            // Handle broadcast results
            const fee = payload
              ? (payload as Record<string, unknown>).fee_result
              : null;
            if (fee) {
              setFeeResult(JSON.stringify(fee));
            }
            setStatus({ type: "success" });
          }
        },
      );

      unlistenError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          if (event.payload.taskId !== currentTaskId) return;
          setStatus({ type: "error", message: event.payload.message });
        },
      );
    })();

    return () => {
      unlistenResult?.();
      unlistenError?.();
    };
  }, [currentTaskId, status.type, handleFetchResult]);

  const handleContractChange = useCallback(async (contractId: string) => {
    setSelectedContractId(contractId);
    setSelectedDocTypeName("");
    setFieldInputs({});
    setOriginalDocument(null);
    setFetchedDocuments([]);
    setFetchedPrice(null);
    setDocumentIdInput("");
    setRecipientIdInput("");
    setPriceInput("");

    if (!contractId) {
      setSelectedContractDetail(null);
      return;
    }
    try {
      const result = await commands.contractGetById(contractId);
      if (result.status === "ok") {
        setSelectedContractDetail(result.data);
        // Auto-select first document type
        if (result.data && result.data.documentTypeNames.length > 0) {
          setSelectedDocTypeName(result.data.documentTypeNames[0] ?? "");
        }
      }
    } catch {
      // Ignore
    }
  }, []);

  const handleDocTypeChange = useCallback((docType: string) => {
    setSelectedDocTypeName(docType);
    setFieldInputs({});
    setOriginalDocument(null);
    setFetchedDocuments([]);
    setFetchedPrice(null);
    setDocumentIdInput("");
    setRecipientIdInput("");
    setPriceInput("");
  }, []);

  const handleIdentityChange = useCallback((id: string) => {
    setRawIdentityId(id);
  }, []);

  const handleFieldChange = useCallback((name: string, value: string) => {
    setFieldInputs((prev) => ({ ...prev, [name]: value }));
  }, []);

  const handleBooleanFieldChange = useCallback(
    (name: string, checked: boolean) => {
      setFieldInputs((prev) => ({
        ...prev,
        [name]: checked ? "true" : "false",
      }));
    },
    [],
  );

  const handleRequestUnlock = useCallback(() => {
    setWalletUnlockOpen(true);
  }, []);

  const handleWalletUnlockResult = useCallback(
    async (result: WalletUnlockResult) => {
      if (result.status === "unlocked" && associatedWallet) {
        setWalletUnlockError(null);
        try {
          await commands.walletNotifyUnlocked(associatedWallet.seedHash);
          setWalletUnlockedHashes(
            (prev) => new Set([...prev, associatedWallet.seedHash]),
          );
        } catch (e) {
          setWalletUnlockError(
            e instanceof Error ? e.message : String(e),
          );
          return;
        }
      }
      setWalletUnlockOpen(false);
    },
    [associatedWallet],
  );

  /** Build document JSON from field inputs. */
  const buildDocumentJson = useCallback((): JsonValue | null => {
    const doc: Record<string, JsonValue> = {};

    for (const [name, schema] of Object.entries(properties)) {
      const value = fieldInputs[name] ?? "";

      if (!value.trim()) {
        if (schema.required) {
          setBackendMessage(`Field "${name}" is required.`);
          return null;
        }
        continue;
      }

      try {
        switch (schema.type) {
          case "integer":
            doc[name] = parseInt(value, 10);
            if (isNaN(doc[name] as number)) {
              setBackendMessage(`${name} must be an integer.`);
              return null;
            }
            break;
          case "number":
            doc[name] = parseFloat(value);
            if (isNaN(doc[name] as number)) {
              setBackendMessage(`${name} must be a number.`);
              return null;
            }
            break;
          case "boolean":
            doc[name] = value === "true" || value === "1" || value === "yes" || value === "on";
            break;
          case "string":
            doc[name] = value;
            break;
          case "byteArray":
            // Passed as hex string — backend will decode
            doc[name] = value;
            break;
          case "identifier":
            // Passed as base58 or hex — backend will decode
            doc[name] = value;
            break;
          case "date":
            doc[name] = parseInt(value, 10);
            if (isNaN(doc[name] as number)) {
              setBackendMessage(`${name} must be a unix timestamp (ms).`);
              return null;
            }
            break;
          case "object":
          case "array":
            doc[name] = JSON.parse(value);
            break;
          default:
            doc[name] = value;
        }
      } catch {
        setBackendMessage(`${name}: invalid value for type "${schema.type}".`);
        return null;
      }
    }

    return doc as JsonValue;
  }, [properties, fieldInputs]);

  /** Fetch documents (for delete, purchase, replace flows). */
  const handleFetch = useCallback(async () => {
    if (!selectedContractId || !selectedDocTypeName) return;

    setBackendMessage(null);
    setStatus({ type: "fetching", startTime: Date.now() });
    setElapsedMs(0);

    try {
      let result;
      if (actionType === "delete" && canFetchOwned && selectedIdentityId) {
        // Fetch owned documents
        result = await commands.documentFetch({
          contractId: selectedContractId,
          documentTypeName: selectedDocTypeName,
          whereClauses: [
            {
              field: "$ownerId",
              operator: "==",
              value: selectedIdentityId,
            },
          ],
          orderByClauses: [],
          limit: 100,
        });
      } else {
        // Fetch by specific document ID
        result = await commands.documentFetch({
          contractId: selectedContractId,
          documentTypeName: selectedDocTypeName,
          whereClauses: [
            {
              field: "$id",
              operator: "==",
              value: documentIdInput.trim(),
            },
          ],
          orderByClauses: [],
          limit: 1,
        });
      }

      if (result.status === "ok") {
        setCurrentTaskId(result.data.taskId);
      } else {
        setStatus({ type: "error", message: result.error });
      }
    } catch (e) {
      setStatus({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [
    selectedContractId,
    selectedDocTypeName,
    actionType,
    canFetchOwned,
    selectedIdentityId,
    documentIdInput,
  ]);

  /** Broadcast the document action. */
  const handleBroadcast = useCallback(async () => {
    if (effectiveKeyId === null) return;

    setBackendMessage(null);
    setStatus({ type: "broadcasting", startTime: Date.now() });
    setElapsedMs(0);

    const tokenPayment: TokenPaymentInfoDto | null = tokenCost
      ? { tokenId: tokenCost.tokenId, amount: tokenCost.tokenAmount }
      : null;

    try {
      let result;

      switch (actionType) {
        case "create": {
          const docJson = buildDocumentJson();
          if (!docJson) {
            setStatus({ type: "input" });
            return;
          }
          result = await commands.documentBroadcast({
            documentJson: docJson,
            tokenPayment,
            documentTypeName: selectedDocTypeName,
            contractId: selectedContractId,
            identityId: selectedIdentityId,
            keyId: effectiveKeyId,
          });
          break;
        }
        case "delete": {
          result = await commands.documentDelete({
            documentId: documentIdInput.trim(),
            documentTypeName: selectedDocTypeName,
            contractId: selectedContractId,
            identityId: selectedIdentityId,
            keyId: effectiveKeyId,
            tokenPayment,
          });
          break;
        }
        case "replace": {
          const docJson = buildDocumentJson();
          if (!docJson) {
            setStatus({ type: "input" });
            return;
          }
          result = await commands.documentReplace({
            documentJson: docJson,
            documentTypeName: selectedDocTypeName,
            contractId: selectedContractId,
            identityId: selectedIdentityId,
            keyId: effectiveKeyId,
            tokenPayment,
          });
          break;
        }
        case "transfer": {
          result = await commands.documentTransfer({
            documentId: documentIdInput.trim(),
            newOwnerId: recipientIdInput.trim(),
            documentTypeName: selectedDocTypeName,
            contractId: selectedContractId,
            identityId: selectedIdentityId,
            keyId: effectiveKeyId,
            tokenPayment,
          });
          break;
        }
        case "purchase": {
          result = await commands.documentPurchase({
            price: fetchedPrice ?? 0,
            documentId: documentIdInput.trim(),
            documentTypeName: selectedDocTypeName,
            contractId: selectedContractId,
            identityId: selectedIdentityId,
            keyId: effectiveKeyId,
            tokenPayment,
          });
          break;
        }
        case "setPrice": {
          const price = parseInt(priceInput.trim(), 10);
          if (isNaN(price)) {
            setBackendMessage("Price must be a valid integer (in credits).");
            setStatus({ type: "input" });
            return;
          }
          result = await commands.documentSetPrice({
            price,
            documentId: documentIdInput.trim(),
            documentTypeName: selectedDocTypeName,
            contractId: selectedContractId,
            identityId: selectedIdentityId,
            keyId: effectiveKeyId,
            tokenPayment,
          });
          break;
        }
      }

      if (result?.status === "ok") {
        setCurrentTaskId(result.data.taskId);
      } else if (result) {
        setStatus({ type: "error", message: result.error });
      }
    } catch (e) {
      setStatus({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, [
    actionType,
    effectiveKeyId,
    selectedContractId,
    selectedDocTypeName,
    selectedIdentityId,
    documentIdInput,
    recipientIdInput,
    priceInput,
    fetchedPrice,
    tokenCost,
    buildDocumentJson,
  ]);

  const handleReset = useCallback(() => {
    setStatus({ type: "input" });
    setFieldInputs({});
    setDocumentIdInput("");
    setRecipientIdInput("");
    setPriceInput("");
    setFetchedPrice(null);
    setFetchedDocuments([]);
    setOriginalDocument(null);
    setBackendMessage(null);
    setFeeResult(null);
    setCurrentTaskId(null);
  }, []);

  const handleDismissError = useCallback(() => {
    setStatus({ type: "input" });
    setBackendMessage(null);
  }, []);

  // ─── Auth key options for advanced mode ──────────────────────────
  const authKeyOptions = useMemo(() => {
    if (!selectedIdentity) return [];
    return selectedIdentity.keys.filter(
      (k) =>
        !k.isDisabled &&
        k.purpose === "AUTHENTICATION" &&
        k.securityLevel !== "MASTER",
    );
  }, [selectedIdentity]);

  // ─── Render ──────────────────────────────────────────────────────

  return (
    <div className="flex flex-1 flex-col overflow-auto" data-testid="document-action-screen">
      <PageHeader
        title={ACTION_LABELS[actionType]}
        breadcrumbs={[
          { label: "Contracts" },
          { label: ACTION_LABELS[actionType] },
        ]}
        actions={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => navigate({ to: "/contracts" })}
            data-testid="back-button"
          >
            <ArrowLeft className="mr-1 h-4 w-4" />
            Back to Contracts
          </Button>
        }
      />

      <div className="flex-1 overflow-auto p-4">
        <Island className="max-w-3xl mx-auto space-y-6 p-6">
          {/* Success state */}
          {status.type === "success" && (
            <div className="text-center space-y-4" data-testid="success-screen">
              <div className="flex items-center justify-center">
                <div className="rounded-full bg-green-100 p-3 dark:bg-green-900/30">
                  <Check className="h-8 w-8 text-green-600 dark:text-green-400" />
                </div>
              </div>
              <h3 className="text-lg font-semibold text-foreground">
                {ACTION_LABELS[actionType]} successful!
              </h3>
              {feeResult && (
                <p className="text-sm text-muted-foreground">
                  Fee paid: {feeResult}
                </p>
              )}
              <div className="flex items-center justify-center gap-3">
                <Button
                  variant="outline"
                  onClick={() => navigate({ to: "/contracts" })}
                  data-testid="back-to-contracts"
                >
                  Back to Contracts
                </Button>
                <Button onClick={handleReset} data-testid="do-another">
                  {ACTION_LABELS[actionType]} Another
                </Button>
              </div>
            </div>
          )}

          {/* Broadcasting state */}
          {status.type === "broadcasting" && (
            <div className="text-center space-y-3" data-testid="broadcasting-state">
              <Loader2 className="mx-auto h-8 w-8 animate-spin text-primary" />
              <p className="text-sm text-muted-foreground">
                Broadcasting... {formatElapsed(elapsedMs)}
              </p>
            </div>
          )}

          {/* Form state */}
          {(status.type === "input" ||
            status.type === "error" ||
            status.type === "fetching" ||
            status.type === "fetched") && (
            <>
              {/* Error banner */}
              {(status.type === "error" || backendMessage) && (
                <div
                  className="rounded-lg border border-destructive/30 bg-destructive/5 p-4 flex items-start justify-between"
                  data-testid="error-banner"
                >
                  <p className="text-sm text-destructive flex-1">
                    {status.type === "error"
                      ? status.message
                      : backendMessage}
                  </p>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="shrink-0 ml-2"
                    onClick={handleDismissError}
                    data-testid="dismiss-error"
                  >
                    Dismiss
                  </Button>
                </div>
              )}

              {/* No identities warning */}
              {identities.length === 0 && (
                <div className="rounded-lg border border-warning/30 bg-warning/5 p-4">
                  <p className="text-sm text-warning">
                    No identities loaded. Load or create an identity first.
                  </p>
                </div>
              )}

              {/* Step 1: Contract & Document Type Selection */}
              <div className="space-y-3">
                <h3 className="text-sm font-semibold text-foreground">
                  1. Select a contract and document type
                </h3>
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1.5">
                    <Label htmlFor="contract-select">Contract</Label>
                    <Select
                      value={selectedContractId}
                      onValueChange={handleContractChange}
                    >
                      <SelectTrigger id="contract-select" data-testid="contract-select">
                        <SelectValue placeholder="Select a contract..." />
                      </SelectTrigger>
                      <SelectContent>
                        {filteredContracts.map((c) => (
                          <SelectItem key={c.id} value={c.id}>
                            {c.alias || c.id.slice(0, 16) + "..."}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="doctype-select">Document Type</Label>
                    <Select
                      value={selectedDocTypeName}
                      onValueChange={handleDocTypeChange}
                      disabled={!selectedContractId || documentTypeNames.length === 0}
                    >
                      <SelectTrigger id="doctype-select" data-testid="doctype-select">
                        <SelectValue placeholder="Select document type..." />
                      </SelectTrigger>
                      <SelectContent>
                        {documentTypeNames.map((name) => (
                          <SelectItem key={name} value={name}>
                            {name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              </div>

              {/* Step 2: Identity & Key Selection */}
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <h3 className="text-sm font-semibold text-foreground">
                    2. Select an identity
                  </h3>
                  <button
                    type="button"
                    className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
                    onClick={() => setShowAdvanced(!showAdvanced)}
                    aria-expanded={showAdvanced}
                    data-testid="advanced-toggle"
                  >
                    {showAdvanced ? (
                      <ChevronUp className="size-3.5" />
                    ) : (
                      <ChevronDown className="size-3.5" />
                    )}
                    Advanced Options
                  </button>
                </div>
                <IdentitySelector
                  value={selectedIdentityId}
                  onChange={handleIdentityChange}
                  identities={identityOptions}
                  label="Identity"
                />
                {selectedIdentity && (
                  <p className="text-xs text-muted-foreground">
                    Balance: {(selectedIdentity.balance / CREDITS_PER_DASH).toFixed(8)} DASH
                  </p>
                )}
                {showAdvanced && selectedIdentity && (
                  <div className="space-y-1.5 pl-4 border-l-2 border-muted">
                    <Label htmlFor="key-select">Signing Key</Label>
                    <Select
                      value={effectiveKeyId !== null ? String(effectiveKeyId) : ""}
                      onValueChange={(v) => setSelectedKeyId(Number(v))}
                    >
                      <SelectTrigger id="key-select" data-testid="key-select">
                        <SelectValue placeholder="Select key..." />
                      </SelectTrigger>
                      <SelectContent>
                        {authKeyOptions.map((k) => (
                          <SelectItem
                            key={k.keyId}
                            value={String(k.keyId)}
                          >
                            Key {k.keyId} — {k.keyType} ({k.securityLevel})
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )}
              </div>

              {/* Step 3: Action-specific fields */}
              <div className="space-y-3">
                <h3 className="text-sm font-semibold text-foreground">
                  3. {getStep3Label(actionType)}
                </h3>

                {actionType === "create" && (
                  <DocumentFieldsForm
                    properties={properties}
                    fieldInputs={fieldInputs}
                    onFieldChange={handleFieldChange}
                    onBooleanChange={handleBooleanFieldChange}
                  />
                )}

                {actionType === "delete" && (
                  <DeleteActionFields
                    documentIdInput={documentIdInput}
                    onDocumentIdChange={setDocumentIdInput}
                    canFetchOwned={canFetchOwned}
                    onFetchOwned={handleFetch}
                    fetchedDocuments={fetchedDocuments}
                    onSelectDocument={setDocumentIdInput}
                    onViewDocument={setViewingDocumentJson}
                    status={status}
                    elapsedMs={elapsedMs}
                  />
                )}

                {actionType === "replace" && (
                  <ReplaceActionFields
                    documentIdInput={documentIdInput}
                    onDocumentIdChange={setDocumentIdInput}
                    onFetch={handleFetch}
                    originalDocument={originalDocument}
                    properties={properties}
                    fieldInputs={fieldInputs}
                    onFieldChange={handleFieldChange}
                    onBooleanChange={handleBooleanFieldChange}
                    status={status}
                    elapsedMs={elapsedMs}
                  />
                )}

                {actionType === "transfer" && (
                  <TransferActionFields
                    documentIdInput={documentIdInput}
                    onDocumentIdChange={setDocumentIdInput}
                    recipientIdInput={recipientIdInput}
                    onRecipientIdChange={setRecipientIdInput}
                  />
                )}

                {actionType === "purchase" && (
                  <PurchaseActionFields
                    documentIdInput={documentIdInput}
                    onDocumentIdChange={setDocumentIdInput}
                    onFetchPrice={handleFetch}
                    fetchedPrice={fetchedPrice}
                    status={status}
                    elapsedMs={elapsedMs}
                  />
                )}

                {actionType === "setPrice" && (
                  <SetPriceActionFields
                    documentIdInput={documentIdInput}
                    onDocumentIdChange={setDocumentIdInput}
                    priceInput={priceInput}
                    onPriceChange={setPriceInput}
                  />
                )}
              </div>

              {/* Token cost info */}
              {tokenCost && (
                <div className="rounded-lg bg-destructive/5 border border-destructive/20 p-3">
                  <p className="text-sm text-destructive">
                    Token cost: {tokenCost.tokenAmount} &quot;{tokenCost.tokenName}&quot; tokens.
                    <br />
                    Tokens will be {tokenCost.effect}.
                    <br />
                    Gas fees will be paid by {tokenCost.gasFeesPaidBy}.
                  </p>
                </div>
              )}

              {/* Wallet locked warning */}
              {walletLocked && (
                <div className="rounded-lg border border-warning/30 bg-warning/5 p-4 flex items-center gap-3">
                  <p className="text-sm text-warning flex-1">
                    Wallet is locked. Please unlock to continue.
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleRequestUnlock}
                    data-testid="unlock-wallet"
                  >
                    Unlock Wallet
                  </Button>
                </div>
              )}

              {/* Fee estimation and broadcast button */}
              {selectedContractId && selectedDocTypeName && (
                <div className="flex items-center justify-between pt-2 border-t border-border">
                  {estimatedFee !== null && (
                    <div>
                      <span className="text-xs text-muted-foreground">
                        Estimated fee:{" "}
                      </span>
                      <span className="text-sm font-mono text-muted-foreground">
                        {formatCreditsAsDash(estimatedFee)} DASH
                      </span>
                    </div>
                  )}
                  <Button
                    onClick={handleBroadcast}
                    disabled={!canBroadcast}
                    data-testid="broadcast-button"
                  >
                    {BROADCAST_LABELS[actionType]}
                  </Button>
                </div>
              )}
            </>
          )}
        </Island>
      </div>

      {/* Wallet unlock dialog */}
      <WalletUnlockDialog
        open={walletUnlockOpen}
        onOpenChange={setWalletUnlockOpen}
        walletAlias={
          associatedWallet?.alias ??
          associatedWallet?.seedHash?.slice(0, 12) ??
          "Wallet"
        }
        passwordHint={associatedWallet?.passwordHint ?? undefined}
        error={walletUnlockError ?? undefined}
        onResult={handleWalletUnlockResult}
      />

      {/* Document view dialog */}
      <Dialog
        open={viewingDocumentJson !== null}
        onOpenChange={(open) => {
          if (!open) setViewingDocumentJson(null);
        }}
      >
        <DialogContent className="max-w-2xl max-h-[80vh] overflow-auto">
          <DialogHeader>
            <DialogTitle>Document</DialogTitle>
          </DialogHeader>
          <pre className="text-xs font-mono bg-muted/50 rounded p-3 overflow-auto max-h-[60vh]">
            {viewingDocumentJson}
          </pre>
        </DialogContent>
      </Dialog>
    </div>
  );
}

// ─── Sub-components ──────────────────────────────────────────────────

function getStep3Label(actionType: DocumentActionType): string {
  switch (actionType) {
    case "create":
      return "Fill out the document type fields";
    case "delete":
      return "Enter document ID to delete";
    case "replace":
      return "Enter document ID and fetch existing document";
    case "transfer":
      return "Transfer document";
    case "purchase":
      return "Enter document ID to purchase";
    case "setPrice":
      return "Set document price";
    default:
      return "";
  }
}

/** Dynamic form fields rendered from document type schema. */
function DocumentFieldsForm({
  properties,
  fieldInputs,
  onFieldChange,
  onBooleanChange,
}: {
  properties: Record<string, PropertySchema>;
  fieldInputs: Record<string, string>;
  onFieldChange: (name: string, value: string) => void;
  onBooleanChange: (name: string, checked: boolean) => void;
}) {
  const entries = Object.entries(properties);
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground" data-testid="no-properties">
        No editable properties for this document type.
      </p>
    );
  }

  return (
    <div className="space-y-3" data-testid="document-fields-form">
      {entries.map(([name, schema]) => (
        <DocumentFieldInput
          key={name}
          name={name}
          schema={schema}
          value={fieldInputs[name] ?? ""}
          onChange={onFieldChange}
          onBooleanChange={onBooleanChange}
        />
      ))}
    </div>
  );
}

function DocumentFieldInput({
  name,
  schema,
  value,
  onChange,
  onBooleanChange,
}: {
  name: string;
  schema: PropertySchema;
  value: string;
  onChange: (name: string, value: string) => void;
  onBooleanChange: (name: string, checked: boolean) => void;
}) {
  const label = schema.required ? `${name} *` : name;

  if (schema.type === "boolean") {
    return (
      <div className="flex items-center gap-2">
        <Checkbox
          id={`field-${name}`}
          checked={value === "true" || value === "1" || value === "yes" || value === "on"}
          onCheckedChange={(checked) => onBooleanChange(name, !!checked)}
          data-testid={`field-${name}`}
        />
        <Label htmlFor={`field-${name}`} className="text-sm">
          {label}
        </Label>
      </div>
    );
  }

  if (
    schema.type === "object" ||
    schema.type === "array"
  ) {
    return (
      <div className="space-y-1">
        <Label htmlFor={`field-${name}`} className="text-sm">
          {label}
        </Label>
        <Textarea
          id={`field-${name}`}
          value={value}
          onChange={(e) => onChange(name, e.target.value)}
          placeholder={getHint(schema)}
          rows={3}
          className="font-mono text-sm"
          data-testid={`field-${name}`}
        />
      </div>
    );
  }

  return (
    <div className="space-y-1">
      <Label htmlFor={`field-${name}`} className="text-sm">
        {label}
      </Label>
      <Input
        id={`field-${name}`}
        value={value}
        onChange={(e) => onChange(name, e.target.value)}
        placeholder={getHint(schema)}
        data-testid={`field-${name}`}
      />
    </div>
  );
}

function getHint(schema: PropertySchema): string {
  switch (schema.type) {
    case "integer":
      return "integer";
    case "number":
      return "floating-point";
    case "string":
      return schema.maxLength ? `max ${schema.maxLength} chars` : "text";
    case "byteArray":
      return "hex or base64";
    case "identifier":
      return "base58 identifier";
    case "date":
      return "unix-ms timestamp";
    case "object":
      return "JSON object";
    case "array":
      return "JSON array";
    default:
      return "";
  }
}

/** Delete action fields: document ID input + fetch owned + fetched documents. */
function DeleteActionFields({
  documentIdInput,
  onDocumentIdChange,
  canFetchOwned,
  onFetchOwned,
  fetchedDocuments,
  onSelectDocument,
  onViewDocument,
  status,
  elapsedMs,
}: {
  documentIdInput: string;
  onDocumentIdChange: (v: string) => void;
  canFetchOwned: boolean;
  onFetchOwned: () => void;
  fetchedDocuments: { id: string; json: string }[];
  onSelectDocument: (id: string) => void;
  onViewDocument: (json: string) => void;
  status: ScreenStatus;
  elapsedMs: number;
}) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <Label htmlFor="document-id">Document ID (Base58)</Label>
        <Input
          id="document-id"
          value={documentIdInput}
          onChange={(e) => onDocumentIdChange(e.target.value)}
          placeholder="Enter document ID..."
          data-testid="document-id-input"
        />
      </div>

      {canFetchOwned && (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">
            This document type has an index on $ownerId, so you can fetch your owned documents.
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={onFetchOwned}
            disabled={status.type === "fetching"}
            data-testid="fetch-owned-button"
          >
            {status.type === "fetching" ? (
              <>
                <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                Fetching... {formatElapsed(elapsedMs)}
              </>
            ) : (
              "Fetch Owned Documents"
            )}
          </Button>
        </div>
      )}

      {!canFetchOwned && (
        <p className="text-xs text-muted-foreground italic">
          (Cannot use Fetch Owned Documents feature — no $ownerId index on this document type.)
        </p>
      )}

      {fetchedDocuments.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-xs font-semibold text-foreground">
            Fetched documents:
          </h4>
          {fetchedDocuments.map((doc) => (
            <div
              key={doc.id}
              className="flex items-center gap-2 text-xs font-mono bg-muted/50 rounded px-2 py-1"
            >
              <span className="truncate flex-1">{doc.id}</span>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-xs"
                onClick={() => onViewDocument(doc.json)}
                data-testid={`view-doc-${doc.id}`}
              >
                <Eye className="h-3 w-3 mr-1" />
                View
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-xs"
                onClick={() => onSelectDocument(doc.id)}
                data-testid={`select-doc-${doc.id}`}
              >
                Select
              </Button>
            </div>
          ))}
        </div>
      )}

      {status.type === "fetched" && fetchedDocuments.length === 0 && (
        <p className="text-xs text-muted-foreground">No owned documents found.</p>
      )}
    </div>
  );
}

/** Replace action fields: document ID + fetch + form. */
function ReplaceActionFields({
  documentIdInput,
  onDocumentIdChange,
  onFetch,
  originalDocument,
  properties,
  fieldInputs,
  onFieldChange,
  onBooleanChange,
  status,
  elapsedMs,
}: {
  documentIdInput: string;
  onDocumentIdChange: (v: string) => void;
  onFetch: () => void;
  originalDocument: JsonValue | null;
  properties: Record<string, PropertySchema>;
  fieldInputs: Record<string, string>;
  onFieldChange: (name: string, value: string) => void;
  onBooleanChange: (name: string, checked: boolean) => void;
  status: ScreenStatus;
  elapsedMs: number;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-end gap-2">
        <div className="flex-1 space-y-1">
          <Label htmlFor="document-id">Document ID (Base58)</Label>
          <Input
            id="document-id"
            value={documentIdInput}
            onChange={(e) => onDocumentIdChange(e.target.value)}
            placeholder="Enter document ID..."
            data-testid="document-id-input"
          />
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={onFetch}
          disabled={!documentIdInput.trim() || status.type === "fetching"}
          data-testid="fetch-document-button"
        >
          {status.type === "fetching" ? (
            <>
              <Loader2 className="mr-1 h-3 w-3 animate-spin" />
              {formatElapsed(elapsedMs)}
            </>
          ) : (
            "Fetch"
          )}
        </Button>
      </div>

      {originalDocument && (
        <>
          <p className="text-xs text-green-600 dark:text-green-400">
            Document fetched successfully.
          </p>
          <h4 className="text-sm font-semibold text-foreground">
            4. Update document fields
          </h4>
          <DocumentFieldsForm
            properties={properties}
            fieldInputs={fieldInputs}
            onFieldChange={onFieldChange}
            onBooleanChange={onBooleanChange}
          />
        </>
      )}
    </div>
  );
}

/** Transfer action fields: document ID + recipient ID. */
function TransferActionFields({
  documentIdInput,
  onDocumentIdChange,
  recipientIdInput,
  onRecipientIdChange,
}: {
  documentIdInput: string;
  onDocumentIdChange: (v: string) => void;
  recipientIdInput: string;
  onRecipientIdChange: (v: string) => void;
}) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <Label htmlFor="document-id">Document ID (Base58)</Label>
        <Input
          id="document-id"
          value={documentIdInput}
          onChange={(e) => onDocumentIdChange(e.target.value)}
          placeholder="Enter document ID..."
          data-testid="document-id-input"
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor="recipient-id">Recipient Identity ID (Base58)</Label>
        <Input
          id="recipient-id"
          value={recipientIdInput}
          onChange={(e) => onRecipientIdChange(e.target.value)}
          placeholder="Enter recipient identity ID..."
          data-testid="recipient-id-input"
        />
      </div>
    </div>
  );
}

/** Purchase action fields: document ID + fetch price + display price. */
function PurchaseActionFields({
  documentIdInput,
  onDocumentIdChange,
  onFetchPrice,
  fetchedPrice,
  status,
  elapsedMs,
}: {
  documentIdInput: string;
  onDocumentIdChange: (v: string) => void;
  onFetchPrice: () => void;
  fetchedPrice: number | null;
  status: ScreenStatus;
  elapsedMs: number;
}) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <Label htmlFor="document-id">Document ID (Base58)</Label>
        <Input
          id="document-id"
          value={documentIdInput}
          onChange={(e) => onDocumentIdChange(e.target.value)}
          placeholder="Enter document ID..."
          data-testid="document-id-input"
        />
      </div>
      <Button
        variant="outline"
        size="sm"
        onClick={onFetchPrice}
        disabled={!documentIdInput.trim() || status.type === "fetching"}
        data-testid="fetch-price-button"
      >
        {status.type === "fetching" ? (
          <>
            <Loader2 className="mr-1 h-3 w-3 animate-spin" />
            Fetching... {formatElapsed(elapsedMs)}
          </>
        ) : (
          "Fetch Document Price"
        )}
      </Button>
      {fetchedPrice !== null && (
        <div className="space-y-1">
          <p className="text-sm text-foreground">
            Document price: <strong>{fetchedPrice}</strong> credits
          </p>
          <p className="text-xs text-muted-foreground italic">
            Note: This is the listed price at the time of fetching. If you press
            the broadcast button, you will attempt to purchase the document at
            this price.
          </p>
        </div>
      )}
    </div>
  );
}

/** Set price action fields: document ID + price input. */
function SetPriceActionFields({
  documentIdInput,
  onDocumentIdChange,
  priceInput,
  onPriceChange,
}: {
  documentIdInput: string;
  onDocumentIdChange: (v: string) => void;
  priceInput: string;
  onPriceChange: (v: string) => void;
}) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <Label htmlFor="document-id">Document ID (Base58)</Label>
        <Input
          id="document-id"
          value={documentIdInput}
          onChange={(e) => onDocumentIdChange(e.target.value)}
          placeholder="Enter document ID..."
          data-testid="document-id-input"
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor="price-input">Price (in credits)</Label>
        <Input
          id="price-input"
          value={priceInput}
          onChange={(e) => onPriceChange(e.target.value)}
          placeholder="Enter price in credits..."
          data-testid="price-input"
        />
      </div>
    </div>
  );
}
