import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type ContractSummaryDto } from "@/bindings";
import { ToolPageLayout } from "@/components/tools/ToolPageLayout";
import { HexInput, decodeToHex, detectFormat } from "@/components/tools/HexInput";
import { JsonViewer } from "@/components/shared/JsonViewer";
import { AlertCircle, X, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

type ParseState =
  | { status: "idle" }
  | { status: "waiting-for-selection" }
  | { status: "error"; message: string }
  | { status: "success"; json: string };

/**
 * Document Visualizer tool screen.
 *
 * Accepts hex, base64, or comma-separated byte data representing a serialized
 * Dash Document. Requires selecting a contract and document type for context
 * before parsing. Displays the deserialized JSON or an error message.
 */
export function DocumentVisualizerScreen() {
  // Contract selection state
  const [contracts, setContracts] = useState<ContractSummaryDto[]>([]);
  const [searchTerm, setSearchTerm] = useState("");
  const [selectedContractId, setSelectedContractId] = useState<string>("");
  const [documentTypeNames, setDocumentTypeNames] = useState<string[]>([]);
  const [selectedDocTypeName, setSelectedDocTypeName] = useState<string>("");

  // Input/output state
  const [inputValue, setInputValue] = useState("");
  const [parseState, setParseState] = useState<ParseState>({
    status: "waiting-for-selection",
  });
  const [isParsing, setIsParsing] = useState(false);

  // Debounce timer ref
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Load contracts on mount
  useEffect(() => {
    async function loadContracts() {
      const result = await commands.contractListLocal();
      if (result.status === "ok") {
        setContracts(result.data);
      }
    }
    loadContracts();
  }, []);

  // When contract selection changes, fetch document type names
  useEffect(() => {
    if (!selectedContractId) {
      setDocumentTypeNames([]);
      setSelectedDocTypeName("");
      return;
    }

    async function loadDocTypes() {
      const result = await commands.contractGetById(selectedContractId);
      if (result.status === "ok" && result.data) {
        setDocumentTypeNames(result.data.documentTypeNames);
      } else {
        setDocumentTypeNames([]);
      }
      setSelectedDocTypeName("");
    }
    loadDocTypes();
  }, [selectedContractId]);

  // Filter contracts by search term
  const filteredContracts = contracts.filter((c) => {
    if (!searchTerm) return true;
    const label = c.alias ?? c.id;
    return label.toLowerCase().includes(searchTerm.toLowerCase());
  });

  // Parse input when we have all required context
  const parseInput = useCallback(
    async (
      value: string,
      contractId: string,
      docTypeName: string,
    ) => {
      if (!contractId || !docTypeName) {
        setParseState({ status: "waiting-for-selection" });
        return;
      }

      const trimmed = value.trim();
      if (!trimmed) {
        setParseState({ status: "idle" });
        return;
      }

      const format = detectFormat(trimmed);
      const hexData = decodeToHex(trimmed, format);

      if (!hexData) {
        setParseState({
          status: "error",
          message:
            "Unable to decode input. Provide valid hex, base64, or comma-separated bytes (0-255).",
        });
        return;
      }

      setIsParsing(true);
      try {
        const result = await commands.parseDocument({
          hexData,
          contractId,
          documentTypeName: docTypeName,
        });
        if (result.status === "ok") {
          setParseState({ status: "success", json: result.data.json });
        } else {
          setParseState({ status: "error", message: result.error });
        }
      } catch (err) {
        setParseState({
          status: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setIsParsing(false);
      }
    },
    [],
  );

  // Debounced parse trigger
  const triggerParse = useCallback(
    (value: string, contractId: string, docTypeName: string) => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
      debounceRef.current = setTimeout(() => {
        parseInput(value, contractId, docTypeName);
      }, 300);
    },
    [parseInput],
  );

  // Cleanup debounce timer on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
      }
    };
  }, []);

  // Re-parse when selections change
  useEffect(() => {
    triggerParse(inputValue, selectedContractId, selectedDocTypeName);
  }, [selectedDocTypeName]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleInputChange = useCallback(
    (value: string) => {
      setInputValue(value);
      triggerParse(value, selectedContractId, selectedDocTypeName);
    },
    [selectedContractId, selectedDocTypeName, triggerParse],
  );

  const handleContractChange = useCallback(
    (contractId: string) => {
      setSelectedContractId(contractId);
      // doc type will be reset by the useEffect above, which will trigger re-parse
    },
    [],
  );

  const handleDocTypeChange = useCallback(
    (docType: string) => {
      setSelectedDocTypeName(docType);
      // re-parse triggered by useEffect watching selectedDocTypeName
    },
    [],
  );

  const dismissError = useCallback(() => {
    setParseState({ status: "idle" });
  }, []);

  // Get display label for a contract
  const contractLabel = (c: ContractSummaryDto) => c.alias ?? c.id;

  // Find selected contract for display
  const selectedContract = contracts.find((c) => c.id === selectedContractId);

  return (
    <ToolPageLayout
      title="Document Visualizer"
      subtitle="Deserialize and inspect Dash Platform documents with contract context"
    >
      <div className="flex flex-col gap-6">
        {/* Contract & Document Type selectors */}
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          {/* Contract selector with search */}
          <div className="space-y-2">
            <Label htmlFor="contract-search">Contract</Label>
            <div className="relative">
              <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
              <Input
                id="contract-search"
                placeholder="Filter contracts..."
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
                className="pl-8"
              />
            </div>
            <Select
              value={selectedContractId}
              onValueChange={handleContractChange}
            >
              <SelectTrigger id="contract-select" aria-label="Select contract">
                <SelectValue placeholder="Select contract…">
                  {selectedContract
                    ? contractLabel(selectedContract)
                    : "Select contract…"}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {filteredContracts.length === 0 ? (
                  <div className="px-2 py-1.5 text-sm text-muted-foreground">
                    No contracts found
                  </div>
                ) : (
                  filteredContracts.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {contractLabel(c)}
                    </SelectItem>
                  ))
                )}
              </SelectContent>
            </Select>
          </div>

          {/* Document type selector */}
          <div className="space-y-2">
            <Label htmlFor="doctype-select">Document Type</Label>
            <Select
              value={selectedDocTypeName}
              onValueChange={handleDocTypeChange}
              disabled={!selectedContractId}
            >
              <SelectTrigger id="doctype-select" aria-label="Select document type">
                <SelectValue
                  placeholder={
                    selectedContractId
                      ? "Select document type…"
                      : "Pick a contract first"
                  }
                />
              </SelectTrigger>
              <SelectContent>
                {documentTypeNames.length === 0 ? (
                  <div className="px-2 py-1.5 text-sm text-muted-foreground">
                    No document types
                  </div>
                ) : (
                  documentTypeNames.map((name) => (
                    <SelectItem key={name} value={name}>
                      {name}
                    </SelectItem>
                  ))
                )}
              </SelectContent>
            </Select>
          </div>
        </div>

        {/* Input section */}
        <HexInput
          value={inputValue}
          onChange={handleInputChange}
          label="Enter hex, base64, or comma-separated integers for Document"
          placeholder="Paste serialized document bytes here..."
          rows={5}
        />

        {/* Output section */}
        <div className="space-y-2">
          <span className="text-sm font-medium text-foreground">Result</span>

          {parseState.status === "waiting-for-selection" && !isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm italic text-muted-foreground">
                Select a contract and document type.
              </span>
            </div>
          )}

          {parseState.status === "idle" && !isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm italic text-muted-foreground">
                Awaiting input…
              </span>
            </div>
          )}

          {isParsing && (
            <div className="rounded-md border bg-muted/30 p-4">
              <span className="text-sm text-muted-foreground">Parsing…</span>
            </div>
          )}

          {parseState.status === "error" && !isParsing && (
            <div
              className={cn(
                "flex items-start gap-3 rounded-md border border-destructive/50 bg-destructive/5 p-4",
              )}
              role="alert"
            >
              <AlertCircle className="mt-0.5 size-4 shrink-0 text-destructive" />
              <div className="min-w-0 flex-1">
                <p className="text-sm text-destructive">{parseState.message}</p>
              </div>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={dismissError}
                aria-label="Dismiss error"
                className="shrink-0 text-destructive hover:text-destructive"
              >
                <X className="size-3.5" />
              </Button>
            </div>
          )}

          {parseState.status === "success" && !isParsing && (
            <JsonViewer
              data={parseState.json}
              expandDepth={4}
              className="max-h-[600px]"
            />
          )}
        </div>
      </div>
    </ToolPageLayout>
  );
}
