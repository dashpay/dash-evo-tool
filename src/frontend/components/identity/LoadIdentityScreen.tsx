import { useCallback, useMemo, useState } from "react";
import { useElapsedTimer } from "@/hooks/useElapsedTimer";
import {
  ArrowLeft,
  AlertCircle,
  CheckCircle2,
  Loader2,
  Search,
  ChevronDown,
  ChevronUp,
  Plus,
  Trash2,
  Shuffle,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { InlineError } from "@/components/feedback/InlineError";
import type { WalletDto, IdentityTypeDto, NetworkDto } from "@/bindings";

// ─── Types ─────────────────────────────────────────────────────────

export type LoadIdentityStatus =
  | { type: "form" }
  | { type: "loading"; startedAt: number; progressMessage?: string }
  | { type: "error"; message: string }
  | { type: "success"; message?: string };

type LoadMode = "byId" | "byWallet" | "byDpnsName";

export interface LoadIdentityScreenProps {
  /** Available HD wallets. */
  wallets: WalletDto[];
  /** Current status. */
  status: LoadIdentityStatus;
  /** Current network (testnet shows extra helper buttons). */
  network?: NetworkDto;
  /** Called to load identity by ID. */
  onLoadById?: (params: {
    identityId: string;
    identityType: IdentityTypeDto;
    alias: string;
    votingPrivateKey: string;
    ownerPrivateKey: string;
    payoutAddressPrivateKey: string;
    keys: string[];
    deriveKeysFromWallets: boolean;
    selectedWalletSeedHash: string | null;
  }) => void;
  /** Called to search identity from wallet at a specific index. */
  onSearchFromWallet?: (params: {
    walletSeedHash: string;
    identityIndex: number;
  }) => void;
  /** Called to batch search identities from wallet up to max index. */
  onSearchUpToIndex?: (params: {
    walletSeedHash: string;
    maxIdentityIndex: number;
  }) => void;
  /** Called to search identity by DPNS name. */
  onSearchByDpnsName?: (params: {
    name: string;
    walletSeedHash: string | null;
  }) => void;
  /** Called to dismiss error. */
  onDismissError?: () => void;
  /** Called to navigate back. */
  onBack?: () => void;
  /** Called after success to load another. */
  onLoadAnother?: () => void;
}

// ─── Helpers ───────────────────────────────────────────────────────

/** Generate a random 64-character hex string for testing. */
function generateRandomHex64(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Validate identity ID: 64 hex chars or valid base58 (26-35 chars). */
function isValidIdentityId(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  // 64 hex chars
  if (/^[0-9a-fA-F]{64}$/.test(trimmed)) return true;
  // Base58 check (relaxed: 20-50 chars of base58 alphabet)
  if (/^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]{20,50}$/.test(trimmed))
    return true;
  return false;
}

// ─── Component ─────────────────────────────────────────────────────

export function LoadIdentityScreen({
  wallets,
  status,
  network,
  onLoadById,
  onSearchFromWallet,
  onSearchUpToIndex,
  onSearchByDpnsName,
  onDismissError,
  onBack,
  onLoadAnother,
}: LoadIdentityScreenProps) {
  const isTestnet = network === "testnet" || network === "devnet" || network === "regtest";

  // ─── Elapsed timer ──────────────────────────────────────────────
  const loadingStartedAt = status.type === "loading" ? status.startedAt : null;
  const elapsed = useElapsedTimer(loadingStartedAt);

  // ─── State ────────────────────────────────────────────────────

  const [mode, setMode] = useState<LoadMode>("byId");
  const [showAdvanced, setShowAdvanced] = useState(false);

  // By ID state
  const [identityId, setIdentityId] = useState("");
  const [identityType, setIdentityType] = useState<IdentityTypeDto>("user");
  const [alias, setAlias] = useState("");
  const [votingPrivateKey, setVotingPrivateKey] = useState("");
  const [ownerPrivateKey, setOwnerPrivateKey] = useState("");
  const [payoutAddressPrivateKey, setPayoutAddressPrivateKey] = useState("");
  const [manualKeys, setManualKeys] = useState<string[]>([]);
  const [deriveFromWallets, setDeriveFromWallets] = useState(false);
  const [selectedWalletSeedHash, setSelectedWalletSeedHash] = useState<
    string | null
  >(null);

  // By wallet state
  const [walletSeedHash, setWalletSeedHash] = useState<string>(
    wallets.length > 0 ? wallets[0]?.seedHash ?? "" : "",
  );
  const [identityIndex, setIdentityIndex] = useState("0");
  const [walletSearchMode, setWalletSearchMode] = useState<
    "specific" | "upTo"
  >("specific");

  // By DPNS state
  const [dpnsName, setDpnsName] = useState("");
  const [dpnsDeriveFromWallet, setDpnsDeriveFromWallet] = useState(false);
  const [dpnsWalletSeedHash, setDpnsWalletSeedHash] = useState<string | null>(
    null,
  );

  // ─── Validation ───────────────────────────────────────────────

  const canSubmitById = useMemo(() => {
    return status.type === "form" && isValidIdentityId(identityId);
  }, [status, identityId]);

  const canSubmitByWallet = useMemo(() => {
    if (status.type !== "form") return false;
    if (!walletSeedHash) return false;
    const idx = parseInt(identityIndex, 10);
    return !isNaN(idx) && idx >= 0;
  }, [status, walletSeedHash, identityIndex]);

  const canSubmitByDpns = useMemo(() => {
    return status.type === "form" && dpnsName.trim().length >= 3;
  }, [status, dpnsName]);

  // ─── Handlers ─────────────────────────────────────────────────

  const handleLoadById = useCallback(() => {
    if (!canSubmitById) return;
    onLoadById?.({
      identityId: identityId.trim(),
      identityType,
      alias: alias.trim(),
      votingPrivateKey: votingPrivateKey.trim(),
      ownerPrivateKey: ownerPrivateKey.trim(),
      payoutAddressPrivateKey: payoutAddressPrivateKey.trim(),
      keys: manualKeys.map((k) => k.trim()).filter(Boolean),
      deriveKeysFromWallets: deriveFromWallets,
      selectedWalletSeedHash: deriveFromWallets ? selectedWalletSeedHash : null,
    });
  }, [
    canSubmitById,
    identityId,
    identityType,
    alias,
    votingPrivateKey,
    ownerPrivateKey,
    payoutAddressPrivateKey,
    manualKeys,
    deriveFromWallets,
    selectedWalletSeedHash,
    onLoadById,
  ]);

  const handleSearchByWallet = useCallback(() => {
    if (!canSubmitByWallet) return;
    const idx = parseInt(identityIndex, 10);
    if (walletSearchMode === "specific") {
      onSearchFromWallet?.({
        walletSeedHash,
        identityIndex: idx,
      });
    } else {
      onSearchUpToIndex?.({
        walletSeedHash,
        maxIdentityIndex: idx,
      });
    }
  }, [
    canSubmitByWallet,
    walletSeedHash,
    identityIndex,
    walletSearchMode,
    onSearchFromWallet,
    onSearchUpToIndex,
  ]);

  const handleSearchByDpns = useCallback(() => {
    if (!canSubmitByDpns) return;
    onSearchByDpnsName?.({
      name: dpnsName.trim(),
      walletSeedHash: dpnsDeriveFromWallet ? dpnsWalletSeedHash : null,
    });
  }, [
    canSubmitByDpns,
    dpnsName,
    dpnsDeriveFromWallet,
    dpnsWalletSeedHash,
    onSearchByDpnsName,
  ]);

  const handleAddManualKey = useCallback(() => {
    setManualKeys((prev) => [...prev, ""]);
  }, []);

  const handleRemoveManualKey = useCallback((index: number) => {
    setManualKeys((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const handleUpdateManualKey = useCallback(
    (index: number, value: string) => {
      setManualKeys((prev) => prev.map((k, i) => (i === index ? value : k)));
    },
    [],
  );

  // ─── Status screens ──────────────────────────────────────────

  if (status.type === "success") {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <CheckCircle2 className="w-16 h-16 text-green-500" aria-hidden />
        <h2 className="text-2xl font-semibold">Identity Loaded!</h2>
        {status.message && (
          <p className="text-muted-foreground text-center max-w-md">
            {status.message}
          </p>
        )}
        <div className="flex gap-3">
          <Button variant="outline" onClick={onLoadAnother}>
            Load Another
          </Button>
          <Button onClick={onBack}>Back to Identities</Button>
        </div>
      </div>
    );
  }

  if (status.type === "error") {
    return (
      <InlineError
        message={status.message}
        heading="Failed to Load Identity"
        onDismiss={onDismissError}
        dismissLabel="Try Again"
        fullScreen
      />
    );
  }

  if (status.type === "loading") {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-6 p-8">
        <Loader2 className="w-12 h-12 animate-spin text-primary" aria-hidden />
        <h2 className="text-xl font-semibold">Searching…</h2>
        {status.progressMessage ? (
          <p className="text-sm text-muted-foreground">
            {status.progressMessage} ({elapsed})
          </p>
        ) : (
          <p className="text-sm text-muted-foreground">
            Time elapsed: {elapsed}
          </p>
        )}
      </div>
    );
  }

  // ─── Form ─────────────────────────────────────────────────────

  return (
    <div className="flex flex-col gap-6 p-1">
      {/* Header */}
      <div className="flex items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={onBack}
          aria-label="Back"
        >
          <ArrowLeft className="w-4 h-4" />
        </Button>
        <h2 className="text-lg font-semibold">Load Existing Identity</h2>
      </div>

      <Separator />

      <Tabs
        value={mode}
        onValueChange={(v) => setMode(v as LoadMode)}
        className="w-full"
      >
        <TabsList className="grid w-full grid-cols-3">
          <TabsTrigger value="byId">By Identity ID</TabsTrigger>
          <TabsTrigger value="byWallet">By Wallet</TabsTrigger>
          <TabsTrigger value="byDpnsName">By DPNS Name</TabsTrigger>
        </TabsList>

        {/* ── By Identity ID ─────────────────────────────────── */}
        <TabsContent value="byId" className="space-y-4 mt-4">
          <div className="space-y-2">
            <Label htmlFor="identity-id-input">
              Identity ID (Hex or Base58)
            </Label>
            <Input
              id="identity-id-input"
              value={identityId}
              onChange={(e) => setIdentityId(e.target.value)}
              placeholder="Enter identity ID…"
            />
            {identityId.trim() && !isValidIdentityId(identityId) && (
              <p className="text-xs text-destructive">
                Enter a valid 64-character hex or Base58 identity ID
              </p>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="alias-input">Alias (optional)</Label>
            <Input
              id="alias-input"
              value={alias}
              onChange={(e) => setAlias(e.target.value)}
              placeholder="Local display name"
            />
          </div>

          {/* Advanced toggle */}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="flex items-center gap-1 text-muted-foreground"
          >
            {showAdvanced ? (
              <ChevronUp className="w-3.5 h-3.5" />
            ) : (
              <ChevronDown className="w-3.5 h-3.5" />
            )}
            {showAdvanced ? "Hide" : "Show"} Advanced Options
          </Button>

          {showAdvanced && (
            <div className="space-y-4 pl-2 border-l-2 border-border">
              {/* Identity type */}
              <div className="space-y-2">
                <Label>Identity Type</Label>
                <Select
                  value={identityType}
                  onValueChange={(v) => setIdentityType(v as IdentityTypeDto)}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="user">User</SelectItem>
                    <SelectItem value="masternode">Masternode</SelectItem>
                    <SelectItem value="evonode">Evonode</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {/* Testnet helper buttons */}
              {isTestnet && (
                <div className="flex items-center gap-2">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          setIdentityId(generateRandomHex64());
                          setIdentityType("evonode");
                          setAlias(`HPMN-${Math.floor(Math.random() * 1000)}`);
                        }}
                        data-testid="fill-random-hpmn"
                      >
                        <Shuffle className="w-3.5 h-3.5 mr-1.5" />
                        Fill Random HPMN
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>
                      Generate a random ProTxHash for testing (Evonode)
                    </TooltipContent>
                  </Tooltip>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          setIdentityId(generateRandomHex64());
                          setIdentityType("masternode");
                          setAlias(`MN-${Math.floor(Math.random() * 1000)}`);
                        }}
                        data-testid="fill-random-masternode"
                      >
                        <Shuffle className="w-3.5 h-3.5 mr-1.5" />
                        Fill Random Masternode
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>
                      Generate a random ProTxHash for testing (Masternode)
                    </TooltipContent>
                  </Tooltip>
                </div>
              )}

              {/* Masternode/Evonode keys */}
              {(identityType === "masternode" ||
                identityType === "evonode") && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>Voting Private Key (Hex or WIF)</Label>
                    <Input
                      value={votingPrivateKey}
                      onChange={(e) => setVotingPrivateKey(e.target.value)}
                      placeholder="Optional"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>Owner Private Key (Hex or WIF)</Label>
                    <Input
                      value={ownerPrivateKey}
                      onChange={(e) => setOwnerPrivateKey(e.target.value)}
                      placeholder="Optional"
                    />
                  </div>
                  {identityType === "evonode" && (
                    <div className="space-y-2">
                      <Label>
                        Payout Address Private Key (Hex or WIF)
                      </Label>
                      <Input
                        value={payoutAddressPrivateKey}
                        onChange={(e) =>
                          setPayoutAddressPrivateKey(e.target.value)
                        }
                        placeholder="Optional"
                      />
                    </div>
                  )}
                </div>
              )}

              {/* Manual keys */}
              {identityType === "user" && (
                <div className="space-y-2">
                  <Label>Private Keys (Hex or WIF)</Label>
                  <p className="text-xs text-muted-foreground">
                    Optional. Keys can be added later from the key management
                    screen.
                  </p>
                  {manualKeys.map((key, i) => (
                    <div key={i} className="flex items-center gap-2">
                      <Input
                        value={key}
                        onChange={(e) =>
                          handleUpdateManualKey(i, e.target.value)
                        }
                        placeholder="Private key…"
                      />
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => handleRemoveManualKey(i)}
                        aria-label={`Remove key ${i + 1}`}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </Button>
                    </div>
                  ))}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleAddManualKey}
                    className="flex items-center gap-1"
                  >
                    <Plus className="w-3.5 h-3.5" />
                    Add key manually
                  </Button>
                </div>
              )}

              {/* Derive from wallets */}
              {wallets.length > 0 && (
                <div className="space-y-2">
                  <div className="flex items-center gap-2">
                    <Checkbox
                      id="derive-from-wallets"
                      checked={deriveFromWallets}
                      onCheckedChange={(v) =>
                        setDeriveFromWallets(v === true)
                      }
                    />
                    <Label htmlFor="derive-from-wallets" className="text-sm">
                      Try to derive private keys from loaded wallet
                    </Label>
                  </div>
                  {deriveFromWallets && (
                    <Select
                      value={selectedWalletSeedHash ?? "all"}
                      onValueChange={(v) =>
                        setSelectedWalletSeedHash(v === "all" ? null : v)
                      }
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="Select wallet" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">
                          All unlocked wallets
                        </SelectItem>
                        {wallets.map((w) => (
                          <SelectItem key={w.seedHash} value={w.seedHash}>
                            {w.alias?.trim() || w.seedHash.slice(0, 10)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                </div>
              )}
            </div>
          )}

          <Button
            onClick={handleLoadById}
            disabled={!canSubmitById}
            className="w-full"
            size="lg"
          >
            <Search className="w-4 h-4 mr-2" />
            Load Identity
          </Button>
        </TabsContent>

        {/* ── By Wallet ──────────────────────────────────────── */}
        <TabsContent value="byWallet" className="space-y-4 mt-4">
          {wallets.length === 0 ? (
            <div className="flex items-center gap-2 p-4 bg-destructive/10 rounded-md">
              <AlertCircle className="w-4 h-4 text-destructive" />
              <p className="text-sm text-destructive">
                No wallets available. Create a wallet first.
              </p>
            </div>
          ) : (
            <>
              <div className="space-y-2">
                <Label>Wallet</Label>
                {wallets.length === 1 ? (
                  <p className="text-sm text-muted-foreground">
                    {wallets[0]?.alias?.trim() ||
                      wallets[0]?.seedHash.slice(0, 10)}
                  </p>
                ) : (
                  <Select
                    value={walletSeedHash}
                    onValueChange={setWalletSeedHash}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder="Select wallet" />
                    </SelectTrigger>
                    <SelectContent>
                      {wallets.map((w) => (
                        <SelectItem key={w.seedHash} value={w.seedHash}>
                          {w.alias?.trim() || w.seedHash.slice(0, 10)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
              </div>

              {/* Advanced: search mode */}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setShowAdvanced(!showAdvanced)}
                className="flex items-center gap-1 text-muted-foreground"
              >
                {showAdvanced ? (
                  <ChevronUp className="w-3.5 h-3.5" />
                ) : (
                  <ChevronDown className="w-3.5 h-3.5" />
                )}
                {showAdvanced ? "Hide" : "Show"} Advanced Options
              </Button>

              {showAdvanced && (
                <div className="space-y-3 pl-2 border-l-2 border-border">
                  <div className="space-y-2">
                    <Label>Search Mode</Label>
                    <Select
                      value={walletSearchMode}
                      onValueChange={(v) =>
                        setWalletSearchMode(v as "specific" | "upTo")
                      }
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="specific">
                          Specific index
                        </SelectItem>
                        <SelectItem value="upTo">
                          All up to index
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              )}

              <div className="space-y-2">
                <Label htmlFor="wallet-index-input">
                  {walletSearchMode === "specific"
                    ? "Identity index"
                    : "Highest index to search (inclusive)"}
                </Label>
                <Input
                  id="wallet-index-input"
                  type="number"
                  min="0"
                  value={identityIndex}
                  onChange={(e) => setIdentityIndex(e.target.value)}
                  placeholder="0"
                />
              </div>

              <Button
                onClick={handleSearchByWallet}
                disabled={!canSubmitByWallet}
                className="w-full"
                size="lg"
              >
                <Search className="w-4 h-4 mr-2" />
                {walletSearchMode === "specific"
                  ? "Search For Identity"
                  : "Search Wallet for Identities"}
              </Button>
            </>
          )}
        </TabsContent>

        {/* ── By DPNS Name ───────────────────────────────────── */}
        <TabsContent value="byDpnsName" className="space-y-4 mt-4">
          <div className="space-y-2">
            <Label htmlFor="dpns-name-input">DPNS Username</Label>
            <div className="flex items-center gap-2">
              <Input
                id="dpns-name-input"
                value={dpnsName}
                onChange={(e) => setDpnsName(e.target.value)}
                placeholder="alice"
              />
              <span className="text-sm text-muted-foreground whitespace-nowrap">
                .dash
              </span>
            </div>
            {dpnsName.trim().length > 0 && dpnsName.trim().length < 3 && (
              <p className="text-xs text-destructive">
                Username must be at least 3 characters
              </p>
            )}
          </div>

          {/* Advanced: derive from wallet */}
          {wallets.length > 0 && (
            <>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setShowAdvanced(!showAdvanced)}
                className="flex items-center gap-1 text-muted-foreground"
              >
                {showAdvanced ? (
                  <ChevronUp className="w-3.5 h-3.5" />
                ) : (
                  <ChevronDown className="w-3.5 h-3.5" />
                )}
                {showAdvanced ? "Hide" : "Show"} Advanced Options
              </Button>

              {showAdvanced && (
                <div className="space-y-2 pl-2 border-l-2 border-border">
                  <div className="flex items-center gap-2">
                    <Checkbox
                      id="dpns-derive-from-wallet"
                      checked={dpnsDeriveFromWallet}
                      onCheckedChange={(v) =>
                        setDpnsDeriveFromWallet(v === true)
                      }
                    />
                    <Label htmlFor="dpns-derive-from-wallet" className="text-sm">
                      Try to derive private keys from loaded wallet
                    </Label>
                  </div>
                  {dpnsDeriveFromWallet && (
                    <Select
                      value={dpnsWalletSeedHash ?? "all"}
                      onValueChange={(v) =>
                        setDpnsWalletSeedHash(v === "all" ? null : v)
                      }
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="Select wallet" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">
                          All unlocked wallets
                        </SelectItem>
                        {wallets.map((w) => (
                          <SelectItem key={w.seedHash} value={w.seedHash}>
                            {w.alias?.trim() || w.seedHash.slice(0, 10)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                </div>
              )}
            </>
          )}

          <Button
            onClick={handleSearchByDpns}
            disabled={!canSubmitByDpns}
            className="w-full"
            size="lg"
          >
            <Search className="w-4 h-4 mr-2" />
            Search by Username
          </Button>
        </TabsContent>
      </Tabs>
    </div>
  );
}
