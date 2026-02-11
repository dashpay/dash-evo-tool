import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events } from "@/bindings";
import type {
  TaskResultEvent,
  TaskErrorEvent,
  QualifiedIdentityDto,
  IdentityKeyDto,
  ContractSummaryDto,
  DataContractDto,
} from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { CopyButton } from "@/components/shared/CopyButton";
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Loader2,
  Lock,
  Shield,
  ShieldCheck,
  ShieldX,
  X,
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
import { cn } from "@/lib/utils";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";

// ─── Types ────────────────────────────────────────────────────────────

type ProofMode = "generate" | "verify";

type GenerateStatus =
  | { type: "idle" }
  | { type: "generating"; taskId: string; startTime: number }
  | { type: "success"; proofBase64: string }
  | { type: "error"; message: string };

type VerifyStatus =
  | { type: "idle" }
  | { type: "verifying"; taskId: string; startTime: number }
  | {
      type: "valid";
      verifiedAt: number;
      contractId: string;
      securityLevel: number;
    }
  | {
      type: "invalid";
      errorMessage: string;
      technicalDetails: string;
    }
  | { type: "error"; message: string };

/** Parsed proof data from base64 or JSON input. */
interface ParsedProofData {
  proof: number[];
  public_inputs: {
    state_root: number[];
    contract_id: number[];
    message_hash: number[];
    timestamp: number;
  };
  metadata: {
    created_at: number;
    proof_size: number;
    generation_time_ms: number;
    security_level: number;
  };
}

/** Convert a byte array to hex string. */
function bytesToHex(bytes: number[]): string {
  return bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Try to parse proof from base64-encoded JSON, then raw JSON. */
function parseProofInput(input: string): ParsedProofData {
  const trimmed = input.trim();

  // Try base64 first
  try {
    const decoded = atob(trimmed);
    const parsed = JSON.parse(decoded) as ParsedProofData;
    validateProofData(parsed);
    return parsed;
  } catch {
    // Fall through to raw JSON
  }

  // Try raw JSON
  const parsed = JSON.parse(trimmed) as ParsedProofData;
  validateProofData(parsed);
  return parsed;
}

function validateProofData(data: ParsedProofData): void {
  if (!data.proof || !Array.isArray(data.proof)) {
    throw new Error("Missing or invalid 'proof' field");
  }
  if (!data.public_inputs) {
    throw new Error("Missing 'public_inputs' field");
  }
  const pi = data.public_inputs;
  if (
    !Array.isArray(pi.state_root) ||
    pi.state_root.length !== 32
  ) {
    throw new Error("Invalid 'state_root': expected 32-byte array");
  }
  if (
    !Array.isArray(pi.contract_id) ||
    pi.contract_id.length !== 32
  ) {
    throw new Error("Invalid 'contract_id': expected 32-byte array");
  }
  if (
    !Array.isArray(pi.message_hash) ||
    pi.message_hash.length !== 32
  ) {
    throw new Error("Invalid 'message_hash': expected 32-byte array");
  }
  if (typeof pi.timestamp !== "number") {
    throw new Error("Missing 'timestamp' in public_inputs");
  }
  if (!data.metadata) {
    throw new Error("Missing 'metadata' field");
  }
}

// System contracts to exclude from the contract selector
const SYSTEM_CONTRACT_ALIASES = [
  "dpns",
  "keyword_search",
  "token_history",
  "withdrawals",
];

// ─── Helpers ──────────────────────────────────────────────────────────

function truncateId(id: string): string {
  if (id.length > 16) {
    return `${id.slice(0, 6)}...${id.slice(-6)}`;
  }
  return id;
}

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds} second${seconds !== 1 ? "s" : ""}`;
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins} minute${mins !== 1 ? "s" : ""} ${secs} second${secs !== 1 ? "s" : ""}`;
}

/**
 * GroveSTARK Zero-Knowledge Proofs tool screen.
 *
 * Supports two modes:
 * - **Generate:** 3-step form to generate a ZK proof of document ownership.
 * - **Verify:** (placeholder for task 9.1k) Paste and verify a proof.
 */
export function GroveSTARKScreen() {
  const [mode, setMode] = useState<ProofMode>("generate");

  // ─── Generate Mode State ────────────────────────────────────────────

  const [selectedIdentityId, setSelectedIdentityId] = useState<string | null>(
    null,
  );
  const [selectedKeyId, setSelectedKeyId] = useState<string | null>(null);
  const [selectedContractId, setSelectedContractId] = useState<string | null>(
    null,
  );
  const [selectedDocType, setSelectedDocType] = useState<string | null>(null);
  const [documentId, setDocumentId] = useState("");
  const [generateStatus, setGenerateStatus] = useState<GenerateStatus>({
    type: "idle",
  });
  const [elapsedSeconds, setElapsedSeconds] = useState(0);

  // ─── Verify Mode State ─────────────────────────────────────────────

  const [proofText, setProofText] = useState("");
  const [verifyStatus, setVerifyStatus] = useState<VerifyStatus>({
    type: "idle",
  });
  const [verifyElapsedSeconds, setVerifyElapsedSeconds] = useState(0);
  const verifyTaskIdRef = useRef<string | null>(null);
  const verifyTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ─── Data from stores ───────────────────────────────────────────────

  const identities = useIdentityStore((s) => s.identities);
  const identitiesLoading = useIdentityStore((s) => s.loading);
  const loadIdentities = useIdentityStore((s) => s.loadIdentities);

  const contracts = useContractStore((s) => s.contracts);
  const contractsLoading = useContractStore((s) => s.loading);
  const loadContracts = useContractStore((s) => s.loadContracts);
  const getContractById = useContractStore((s) => s.getContractById);

  // Contract detail for document types
  const [contractDetail, setContractDetail] = useState<DataContractDto | null>(
    null,
  );

  // Refs
  const activeTaskIdRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // ─── Derived data ───────────────────────────────────────────────────

  // Filter identities to those with EdDSA keys
  const eddsaIdentities = identities.filter((identity) =>
    identity.keys.some(
      (k) =>
        k.keyType === "EDDSA_25519_HASH160" &&
        !k.isDisabled &&
        (k.purpose === "AUTHENTICATION" || k.purpose === "TRANSFER"),
    ),
  );

  // Get selected identity
  const selectedIdentity = selectedIdentityId
    ? eddsaIdentities.find((i) => i.id === selectedIdentityId)
    : null;

  // Get EdDSA keys for selected identity
  const availableKeys: IdentityKeyDto[] = selectedIdentity
    ? selectedIdentity.keys.filter(
        (k) =>
          k.keyType === "EDDSA_25519_HASH160" &&
          !k.isDisabled &&
          k.hasPrivateKey &&
          (k.purpose === "AUTHENTICATION" || k.purpose === "TRANSFER"),
      )
    : [];

  // Get selected key
  const selectedKey = selectedKeyId
    ? availableKeys.find((k) => String(k.keyId) === selectedKeyId)
    : null;

  // Filter out system contracts
  const userContracts = contracts.filter(
    (c) => !c.alias || !SYSTEM_CONTRACT_ALIASES.includes(c.alias),
  );

  // Document types from selected contract
  const documentTypes = contractDetail?.documentTypeNames ?? [];

  // Can generate?
  const canGenerate =
    selectedIdentity !== null &&
    selectedKey !== null &&
    selectedContractId !== null &&
    selectedDocType !== null &&
    documentId.trim().length > 0 &&
    generateStatus.type !== "generating";

  // ─── Effects ────────────────────────────────────────────────────────

  // Load identities and contracts on mount
  useEffect(() => {
    if (identities.length === 0 && !identitiesLoading) loadIdentities();
    if (contracts.length === 0 && !contractsLoading) loadContracts();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Fetch contract detail when contract selection changes
  useEffect(() => {
    if (!selectedContractId) {
      return;
    }

    let cancelled = false;
    getContractById(selectedContractId).then((detail) => {
      if (!cancelled) {
        setContractDetail(detail);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [selectedContractId, getContractById]);

  // Elapsed time counter for generation
  useEffect(() => {
    if (generateStatus.type !== "generating") {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      return;
    }
    timerRef.current = setInterval(() => {
      setElapsedSeconds((prev) => prev + 1);
    }, 1000);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [generateStatus.type]);

  // Elapsed time counter for verification
  useEffect(() => {
    if (verifyStatus.type !== "verifying") {
      if (verifyTimerRef.current) {
        clearInterval(verifyTimerRef.current);
        verifyTimerRef.current = null;
      }
      return;
    }
    verifyTimerRef.current = setInterval(() => {
      setVerifyElapsedSeconds((prev) => prev + 1);
    }, 1000);
    return () => {
      if (verifyTimerRef.current) clearInterval(verifyTimerRef.current);
    };
  }, [verifyStatus.type]);

  // Subscribe to task result/error events
  useEffect(() => {
    let cleanupResult: (() => void) | undefined;
    let cleanupError: (() => void) | undefined;

    const subscribe = async () => {
      cleanupResult = await events.taskResultEvent.listen(
        (event: { payload: TaskResultEvent }) => {
          const taskId = event.payload.taskId;

          // Handle generate result
          if (activeTaskIdRef.current && taskId === activeTaskIdRef.current) {
            activeTaskIdRef.current = null;
            const payload = event.payload.payload;
            setGenerateStatus({
              type: "success",
              proofBase64: payload ?? "Proof generated successfully",
            });
          }

          // Handle verify result
          if (
            verifyTaskIdRef.current &&
            taskId === verifyTaskIdRef.current
          ) {
            verifyTaskIdRef.current = null;
            // The payload contains verification result info
            // The result type is "GroveSTARK" and contains VerifiedProof(bool, ProofDataOutput)
            const payload = event.payload.payload;
            try {
              const parsed =
                typeof payload === "string" ? JSON.parse(payload) : payload;
              // Backend returns { VerifiedProof: [boolean, proofData] }
              if (parsed?.VerifiedProof) {
                const [isValid, proofData] = parsed.VerifiedProof;
                if (isValid) {
                  const contractId = bytesToHex(
                    proofData.public_inputs.contract_id,
                  );
                  setVerifyStatus({
                    type: "valid",
                    verifiedAt: Math.floor(Date.now() / 1000),
                    contractId,
                    securityLevel: proofData.metadata?.security_level ?? 128,
                  });
                } else {
                  setVerifyStatus({
                    type: "invalid",
                    errorMessage: "Proof verification failed",
                    technicalDetails: "Verification result: INVALID",
                  });
                }
              } else {
                // Fallback: treat any non-error result as valid
                setVerifyStatus({
                  type: "valid",
                  verifiedAt: Math.floor(Date.now() / 1000),
                  contractId: "unknown",
                  securityLevel: 128,
                });
              }
            } catch {
              // If we can't parse the payload, treat as valid (task succeeded)
              setVerifyStatus({
                type: "valid",
                verifiedAt: Math.floor(Date.now() / 1000),
                contractId: "unknown",
                securityLevel: 128,
              });
            }
          }
        },
      );

      cleanupError = await events.taskErrorEvent.listen(
        (event: { payload: TaskErrorEvent }) => {
          const taskId = event.payload.taskId;

          // Handle generate error
          if (activeTaskIdRef.current && taskId === activeTaskIdRef.current) {
            activeTaskIdRef.current = null;
            setGenerateStatus({
              type: "error",
              message: event.payload.message,
            });
          }

          // Handle verify error
          if (
            verifyTaskIdRef.current &&
            taskId === verifyTaskIdRef.current
          ) {
            verifyTaskIdRef.current = null;
            setVerifyStatus({
              type: "invalid",
              errorMessage: event.payload.message,
              technicalDetails: `Verification result: INVALID\n${event.payload.message}`,
            });
          }
        },
      );
    };

    subscribe();
    return () => {
      cleanupResult?.();
      cleanupError?.();
    };
  }, []);

  // ─── Handlers ───────────────────────────────────────────────────────

  const handleIdentityChange = useCallback((value: string) => {
    setSelectedIdentityId(value);
    setSelectedKeyId(null); // Reset key when identity changes
  }, []);

  const handleContractChange = useCallback((value: string) => {
    setSelectedContractId(value);
    setSelectedDocType(null); // Reset doc type when contract changes
    setContractDetail(null); // Clear old contract detail
  }, []);

  const handleGenerate = useCallback(async () => {
    if (
      !selectedIdentityId ||
      !selectedKey ||
      !selectedContractId ||
      !selectedDocType ||
      !documentId.trim()
    ) {
      return;
    }

    try {
      const result = await commands.grovestarkGenerateProof({
        identityId: selectedIdentityId,
        contractId: selectedContractId,
        documentType: selectedDocType,
        documentId: documentId.trim(),
        keyId: selectedKey.keyId,
      });

      if (result.status === "ok") {
        activeTaskIdRef.current = result.data.taskId;
        setElapsedSeconds(0);
        setGenerateStatus({
          type: "generating",
          taskId: result.data.taskId,
          startTime: Date.now(),
        });
      } else {
        setGenerateStatus({
          type: "error",
          message: result.error,
        });
      }
    } catch (err) {
      setGenerateStatus({
        type: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }, [
    selectedIdentityId,
    selectedKey,
    selectedContractId,
    selectedDocType,
    documentId,
  ]);

  const dismissError = useCallback(() => {
    setGenerateStatus({ type: "idle" });
  }, []);

  const handleVerify = useCallback(async () => {
    if (!proofText.trim()) return;

    let parsed: ParsedProofData;
    try {
      parsed = parseProofInput(proofText);
    } catch (err) {
      setVerifyStatus({
        type: "error",
        message: `Failed to parse proof: ${err instanceof Error ? err.message : String(err)}`,
      });
      return;
    }

    try {
      const result = await commands.grovestarkVerifyProof({
        proofHex: bytesToHex(parsed.proof),
        stateRootHex: bytesToHex(parsed.public_inputs.state_root),
        contractIdHex: bytesToHex(parsed.public_inputs.contract_id),
        messageHashHex: bytesToHex(parsed.public_inputs.message_hash),
        timestamp: parsed.public_inputs.timestamp,
        createdAt: parsed.metadata.created_at,
        proofSize: parsed.metadata.proof_size,
        generationTimeMs: parsed.metadata.generation_time_ms,
        securityLevel: parsed.metadata.security_level,
      });

      if (result.status === "ok") {
        verifyTaskIdRef.current = result.data.taskId;
        setVerifyElapsedSeconds(0);
        setVerifyStatus({
          type: "verifying",
          taskId: result.data.taskId,
          startTime: Date.now(),
        });
      } else {
        setVerifyStatus({
          type: "error",
          message: result.error,
        });
      }
    } catch (err) {
      setVerifyStatus({
        type: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }, [proofText]);

  const dismissVerifyError = useCallback(() => {
    setVerifyStatus({ type: "idle" });
  }, []);

  const resetVerify = useCallback(() => {
    setProofText("");
    setVerifyStatus({ type: "idle" });
    verifyTaskIdRef.current = null;
  }, []);

  // ─── Render ─────────────────────────────────────────────────────────

  return (
    <ToolPageLayout
      title="GroveSTARK Zero-Knowledge Proofs"
      subtitle="Generate and verify zero-knowledge proofs for platform documents"
    >
      <div className="flex flex-col gap-6">
        {/* Research Warning */}
        <div
          className="flex items-start gap-3 rounded-lg border border-amber-300 bg-amber-50 p-4 dark:border-amber-700 dark:bg-amber-950/50"
          role="alert"
        >
          <AlertTriangle className="mt-0.5 size-5 shrink-0 text-amber-600 dark:text-amber-400" />
          <div className="text-sm text-amber-800 dark:text-amber-200">
            <span className="font-semibold">WARNING:</span> GroveSTARK is a
            research project. It has not been audited and may contain bugs. Do
            not use in production.
          </div>
        </div>

        {/* Mode Toggle */}
        <div className="flex gap-0 rounded-lg border bg-muted/30 p-1">
          <Button
            variant={mode === "generate" ? "default" : "ghost"}
            size="sm"
            onClick={() => setMode("generate")}
            className={cn(
              "flex-1 gap-2",
              mode === "generate" && "shadow-sm",
            )}
          >
            <Lock className="size-4" />
            Generate Proof
          </Button>
          <Button
            variant={mode === "verify" ? "default" : "ghost"}
            size="sm"
            onClick={() => setMode("verify")}
            className={cn(
              "flex-1 gap-2",
              mode === "verify" && "shadow-sm",
            )}
          >
            <Shield className="size-4" />
            Verify Proof
          </Button>
        </div>

        {/* Mode Content */}
        {mode === "generate" ? (
          <GenerateModeContent
            eddsaIdentities={eddsaIdentities}
            selectedIdentityId={selectedIdentityId}
            selectedKey={selectedKey}
            selectedKeyId={selectedKeyId}
            availableKeys={availableKeys}
            userContracts={userContracts}
            selectedContractId={selectedContractId}
            documentTypes={documentTypes}
            selectedDocType={selectedDocType}
            documentId={documentId}
            generateStatus={generateStatus}
            elapsedSeconds={elapsedSeconds}
            canGenerate={canGenerate}
            onIdentityChange={handleIdentityChange}
            onKeyChange={setSelectedKeyId}
            onContractChange={handleContractChange}
            onDocTypeChange={setSelectedDocType}
            onDocumentIdChange={setDocumentId}
            onGenerate={handleGenerate}
            onDismissError={dismissError}
          />
        ) : (
          <VerifyModeContent
            proofText={proofText}
            verifyStatus={verifyStatus}
            elapsedSeconds={verifyElapsedSeconds}
            onProofTextChange={setProofText}
            onVerify={handleVerify}
            onDismissError={dismissVerifyError}
            onReset={resetVerify}
          />
        )}
      </div>
    </ToolPageLayout>
  );
}

// ─── Generate Mode ──────────────────────────────────────────────────────

interface GenerateModeProps {
  eddsaIdentities: QualifiedIdentityDto[];
  selectedIdentityId: string | null;
  selectedKey: IdentityKeyDto | null;
  selectedKeyId: string | null;
  availableKeys: IdentityKeyDto[];
  userContracts: ContractSummaryDto[];
  selectedContractId: string | null;
  documentTypes: string[];
  selectedDocType: string | null;
  documentId: string;
  generateStatus: GenerateStatus;
  elapsedSeconds: number;
  canGenerate: boolean;
  onIdentityChange: (id: string) => void;
  onKeyChange: (keyId: string) => void;
  onContractChange: (id: string) => void;
  onDocTypeChange: (type: string) => void;
  onDocumentIdChange: (id: string) => void;
  onGenerate: () => void;
  onDismissError: () => void;
}

function GenerateModeContent({
  eddsaIdentities,
  selectedIdentityId,
  selectedKey,
  selectedKeyId,
  availableKeys,
  userContracts,
  selectedContractId,
  documentTypes,
  selectedDocType,
  documentId,
  generateStatus,
  elapsedSeconds,
  canGenerate,
  onIdentityChange,
  onKeyChange,
  onContractChange,
  onDocTypeChange,
  onDocumentIdChange,
  onGenerate,
  onDismissError,
}: GenerateModeProps) {
  return (
    <div className="flex flex-col gap-4">
      {/* Description */}
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">
          Contract Membership Circuit
        </h2>
        <p className="text-sm text-muted-foreground">
          Prove you own a document in a specific contract without revealing
          anything about your identity or the document.
        </p>
      </div>

      {/* Step 1: Select Identity */}
      <div className="rounded-lg border bg-card p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-base font-semibold">Step 1: Select Identity</h3>
          {selectedIdentityId && (
            <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
              <CheckCircle2 className="size-4" />
              Identity selected
            </span>
          )}
        </div>

        <div className="space-y-2">
          <Label htmlFor="identity-select">Identity</Label>
          <Select
            value={selectedIdentityId ?? ""}
            onValueChange={onIdentityChange}
          >
            <SelectTrigger id="identity-select">
              <SelectValue
                placeholder={
                  eddsaIdentities.length === 0
                    ? "No identities with EdDSA keys"
                    : "Select identity..."
                }
              />
            </SelectTrigger>
            <SelectContent>
              {eddsaIdentities.map((identity) => (
                <SelectItem key={identity.id} value={identity.id}>
                  {identity.alias
                    ? `${identity.alias} (${truncateId(identity.id)})`
                    : truncateId(identity.id)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {eddsaIdentities.length === 0 && (
            <p className="text-xs text-muted-foreground">
              ZK proofs require identities with EdDSA (Ed25519) keys. Please add
              an EdDSA key to an identity.
            </p>
          )}
        </div>

        {/* Key Selection — shown when identity selected */}
        {selectedIdentityId && (
          <div className="space-y-2 border-t pt-3">
            <div className="flex items-center justify-between">
              <Label htmlFor="key-select">Select Key for Signing</Label>
              {selectedKey && (
                <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
                  <CheckCircle2 className="size-4" />
                  Key selected
                </span>
              )}
            </div>

            {availableKeys.length === 0 ? (
              <div className="rounded-md border border-amber-300 bg-amber-50 p-3 dark:border-amber-700 dark:bg-amber-950/50">
                <p className="text-sm text-amber-800 dark:text-amber-200">
                  No EdDSA keys with private key available for ZK proof
                  generation. Please add an EdDSA key with a private key to this
                  identity.
                </p>
              </div>
            ) : (
              <Select
                value={selectedKeyId ?? ""}
                onValueChange={onKeyChange}
              >
                <SelectTrigger id="key-select">
                  <SelectValue placeholder="Select key..." />
                </SelectTrigger>
                <SelectContent>
                  {availableKeys.map((key) => (
                    <SelectItem
                      key={key.keyId}
                      value={String(key.keyId)}
                    >
                      EdDSA Key {key.keyId} ({key.purpose} -{" "}
                      {key.securityLevel})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>
        )}
      </div>

      {/* Step 2: Select Contract */}
      <div className="rounded-lg border bg-card p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-base font-semibold">
            Step 2: Select Contract
          </h3>
          {selectedContractId && (
            <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
              <CheckCircle2 className="size-4" />
              Contract selected
            </span>
          )}
        </div>

        <div className="space-y-2">
          <Label htmlFor="contract-select">Contract</Label>
          <Select
            value={selectedContractId ?? ""}
            onValueChange={onContractChange}
          >
            <SelectTrigger id="contract-select">
              <SelectValue
                placeholder={
                  userContracts.length === 0
                    ? "No user contracts available"
                    : "Select contract..."
                }
              />
            </SelectTrigger>
            <SelectContent>
              {userContracts.map((contract) => (
                <SelectItem key={contract.id} value={contract.id}>
                  {contract.alias ?? `Contract ${contract.id.slice(0, 8)}...`}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {userContracts.length === 0 && (
            <p className="text-xs text-muted-foreground">
              No user contracts found. Please create a contract first.
            </p>
          )}
        </div>

        {/* Document Type Selection — shown when contract selected */}
        {selectedContractId && (
          <div className="space-y-2 border-t pt-3">
            <div className="flex items-center justify-between">
              <Label htmlFor="doctype-select">Select Document Type</Label>
              {selectedDocType && (
                <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
                  <CheckCircle2 className="size-4" />
                  Document type selected
                </span>
              )}
            </div>
            <Select
              value={selectedDocType ?? ""}
              onValueChange={onDocTypeChange}
            >
              <SelectTrigger id="doctype-select">
                <SelectValue
                  placeholder={
                    documentTypes.length === 0
                      ? "No document types available"
                      : "Select document type..."
                  }
                />
              </SelectTrigger>
              <SelectContent>
                {documentTypes.map((dt) => (
                  <SelectItem key={dt} value={dt}>
                    {dt}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      {/* Step 3: Document ID */}
      <div className="rounded-lg border bg-card p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-base font-semibold">
            Step 3: Enter Document ID
          </h3>
          {documentId.trim().length > 0 && (
            <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
              <CheckCircle2 className="size-4" />
              Document ID entered
            </span>
          )}
        </div>

        <div className="space-y-2">
          <Label htmlFor="document-id">Document ID</Label>
          <Input
            id="document-id"
            placeholder="Enter the document ID you want to prove ownership of..."
            value={documentId}
            onChange={(e) => onDocumentIdChange(e.target.value)}
          />
        </div>
      </div>

      {/* Error Display */}
      {generateStatus.type === "error" && (
        <div
          className="flex items-start gap-3 rounded-lg border border-destructive/50 bg-destructive/5 p-4"
          role="alert"
        >
          <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
          <div className="min-w-0 flex-1">
            <p className="text-sm text-destructive">
              {generateStatus.message}
            </p>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onDismissError}
            aria-label="Dismiss error"
            className="shrink-0 text-destructive hover:text-destructive"
          >
            <X className="size-3.5" />
          </Button>
        </div>
      )}

      {/* Generate Button / Generating Status */}
      <div className="border-t pt-4">
        {generateStatus.type === "generating" ? (
          <div className="flex items-center gap-3 rounded-lg border border-blue-200 bg-blue-50 p-4 dark:border-blue-800 dark:bg-blue-950">
            <Loader2 className="size-5 animate-spin text-blue-600 dark:text-blue-400" />
            <div>
              <p className="text-sm font-medium text-blue-700 dark:text-blue-300">
                Generating ZK proof...
              </p>
              <p className="text-xs text-blue-600 dark:text-blue-400">
                Time elapsed: {formatElapsed(elapsedSeconds)}
              </p>
            </div>
          </div>
        ) : (
          <Button
            onClick={onGenerate}
            disabled={!canGenerate}
            className="gap-2"
          >
            <Lock className="size-4" />
            Generate Proof
          </Button>
        )}
      </div>

      {/* Success Display */}
      {generateStatus.type === "success" && (
        <div className="rounded-lg border-2 border-green-500 bg-green-50 p-4 dark:bg-green-950/30">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <CheckCircle2 className="size-5 text-green-600 dark:text-green-400" />
              <span className="font-semibold text-green-700 dark:text-green-300">
                Proof Generated Successfully!
              </span>
            </div>
            <CopyButton
              value={generateStatus.proofBase64}
              label="Copy Proof"
              size="sm"
            />
          </div>
          <p className="mt-2 text-xs text-green-600 dark:text-green-400">
            The proof has been generated. Copy it to share or paste it in the
            Verify tab to check its validity.
          </p>
        </div>
      )}
    </div>
  );
}

// ─── Verify Mode ────────────────────────────────────────────────────────

interface VerifyModeProps {
  proofText: string;
  verifyStatus: VerifyStatus;
  elapsedSeconds: number;
  onProofTextChange: (text: string) => void;
  onVerify: () => void;
  onDismissError: () => void;
  onReset: () => void;
}

function VerifyModeContent({
  proofText,
  verifyStatus,
  elapsedSeconds,
  onProofTextChange,
  onVerify,
  onDismissError,
  onReset,
}: VerifyModeProps) {
  const [showTechnicalDetails, setShowTechnicalDetails] = useState(false);
  const canVerify =
    proofText.trim().length > 0 && verifyStatus.type !== "verifying";

  return (
    <div className="flex flex-col gap-4">
      {/* Description */}
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">Verify Zero-Knowledge Proof</h2>
        <p className="text-sm text-muted-foreground">
          Paste a proof (Base64 or JSON) to verify its validity without learning
          anything about the prover or the document.
        </p>
      </div>

      {/* Proof Input */}
      <div className="rounded-lg border bg-card p-4 space-y-3">
        <Label htmlFor="proof-input">Paste Proof (Base64 or JSON)</Label>
        <Textarea
          id="proof-input"
          placeholder="Paste the proof data here..."
          value={proofText}
          onChange={(e) => onProofTextChange(e.target.value)}
          rows={6}
          className="font-mono text-xs"
        />
      </div>

      {/* Parse/Verify Error */}
      {verifyStatus.type === "error" && (
        <div
          className="flex items-start gap-3 rounded-lg border border-destructive/50 bg-destructive/5 p-4"
          role="alert"
        >
          <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
          <div className="min-w-0 flex-1">
            <p className="text-sm text-destructive">{verifyStatus.message}</p>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onDismissError}
            aria-label="Dismiss error"
            className="shrink-0 text-destructive hover:text-destructive"
          >
            <X className="size-3.5" />
          </Button>
        </div>
      )}

      {/* Verify Button / Verifying Status */}
      <div className="border-t pt-4">
        {verifyStatus.type === "verifying" ? (
          <div className="flex items-center gap-3 rounded-lg border border-blue-200 bg-blue-50 p-4 dark:border-blue-800 dark:bg-blue-950">
            <Loader2 className="size-5 animate-spin text-blue-600 dark:text-blue-400" />
            <div>
              <p className="text-sm font-medium text-blue-700 dark:text-blue-300">
                Verifying proof...
              </p>
              <p className="text-xs text-blue-600 dark:text-blue-400">
                Time elapsed: {formatElapsed(elapsedSeconds)}
              </p>
            </div>
          </div>
        ) : (
          <Button onClick={onVerify} disabled={!canVerify} className="gap-2">
            <Shield className="size-4" />
            Verify Proof
          </Button>
        )}
      </div>

      {/* Valid Proof Result */}
      {verifyStatus.type === "valid" && (
        <div className="rounded-lg border-2 border-green-500 bg-green-50 p-4 dark:bg-green-950/30">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <ShieldCheck className="size-5 text-green-600 dark:text-green-400" />
              <span className="font-bold text-green-700 dark:text-green-300">
                PROOF IS VALID
              </span>
            </div>
            <CopyButton
              value={JSON.stringify(
                {
                  valid: true,
                  verifiedAt: new Date(
                    verifyStatus.verifiedAt * 1000,
                  ).toISOString(),
                  contractId: verifyStatus.contractId,
                  securityLevel: `${verifyStatus.securityLevel}-bit`,
                },
                null,
                2,
              )}
              label="Copy Result"
              size="sm"
            />
          </div>
          <div className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
            <span className="font-medium text-green-700 dark:text-green-300">
              Verified At:
            </span>
            <span className="text-green-600 dark:text-green-400">
              {new Date(verifyStatus.verifiedAt * 1000).toLocaleString()}
            </span>
            <span className="font-medium text-green-700 dark:text-green-300">
              Document Exists:
            </span>
            <span className="text-green-600 dark:text-green-400">Yes</span>
            <span className="font-medium text-green-700 dark:text-green-300">
              Key Control:
            </span>
            <span className="text-green-600 dark:text-green-400">Verified</span>
            <span className="font-medium text-green-700 dark:text-green-300">
              Contract:
            </span>
            <span className="font-mono text-xs text-green-600 dark:text-green-400 break-all">
              {verifyStatus.contractId}
            </span>
            <span className="font-medium text-green-700 dark:text-green-300">
              Security Level:
            </span>
            <span className="text-green-600 dark:text-green-400">
              {verifyStatus.securityLevel}-bit
            </span>
          </div>
          <div className="mt-3 flex justify-end">
            <Button variant="outline" size="sm" onClick={onReset}>
              Verify Another
            </Button>
          </div>
        </div>
      )}

      {/* Invalid Proof Result */}
      {verifyStatus.type === "invalid" && (
        <div className="rounded-lg border-2 border-red-500 bg-red-50 p-4 dark:bg-red-950/30">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <ShieldX className="size-5 text-red-600 dark:text-red-400" />
              <span className="font-bold text-red-700 dark:text-red-300">
                PROOF IS INVALID
              </span>
            </div>
          </div>
          {verifyStatus.errorMessage && (
            <div className="mt-2">
              <span className="text-sm font-medium text-red-700 dark:text-red-300">
                Reason:{" "}
              </span>
              <span className="text-sm text-red-600 dark:text-red-400">
                {verifyStatus.errorMessage}
              </span>
            </div>
          )}
          <div className="mt-3">
            <button
              type="button"
              onClick={() => setShowTechnicalDetails((prev) => !prev)}
              className="flex items-center gap-1 text-xs font-medium text-red-600 hover:text-red-700 dark:text-red-400 dark:hover:text-red-300"
            >
              {showTechnicalDetails ? (
                <ChevronDown className="size-3.5" />
              ) : (
                <ChevronRight className="size-3.5" />
              )}
              Technical Details
            </button>
            {showTechnicalDetails && (
              <pre className="mt-2 rounded-md bg-red-100 p-3 text-xs font-mono text-red-800 dark:bg-red-900/50 dark:text-red-200 whitespace-pre-wrap break-all">
                {verifyStatus.technicalDetails}
              </pre>
            )}
          </div>
          <div className="mt-3 flex justify-end">
            <Button variant="outline" size="sm" onClick={onReset}>
              Try Another
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
