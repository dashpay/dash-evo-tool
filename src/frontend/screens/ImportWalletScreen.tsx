import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  Check,
  Eye,
  EyeOff,
  Key,
  RefreshCw,
  Wallet,
} from "lucide-react";
import { validateMnemonic } from "@scure/bip39";
import { wordlist as englishWordlist } from "@scure/bip39/wordlists/english.js";
import { wordlist as spanishWordlist } from "@scure/bip39/wordlists/spanish.js";
import { wordlist as frenchWordlist } from "@scure/bip39/wordlists/french.js";
import { wordlist as italianWordlist } from "@scure/bip39/wordlists/italian.js";
import { wordlist as portugueseWordlist } from "@scure/bip39/wordlists/portuguese.js";
import { toast } from "sonner";
import { toastError } from "@/lib/toastError";
import { Island } from "@/components/layout";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { commands } from "@/bindings";
import { cn } from "@/lib/utils";
import { useWalletStore } from "@/stores/walletStore";
import { PasswordStrengthMeter } from "@/components/shared/PasswordStrengthMeter";

// ─── Constants ────────────────────────────────────────────────────

const WORD_COUNTS = [12, 15, 18, 21, 24] as const;
type WordCount = (typeof WORD_COUNTS)[number];

type ImportMode = "mnemonic" | "privateKey";

const BIP39_LANGUAGES = [
  "English",
  "Spanish",
  "French",
  "Italian",
  "Portuguese",
] as const;
type Bip39Language = (typeof BIP39_LANGUAGES)[number];

const WORDLISTS: Record<Bip39Language, string[]> = {
  English: englishWordlist,
  Spanish: spanishWordlist,
  French: frenchWordlist,
  Italian: italianWordlist,
  Portuguese: portugueseWordlist,
};

function validateSeedPhrase(words: string[], wl: string[]): {
  valid: boolean;
  error: string | null;
} {
  const filled = words.filter((w) => w.trim().length > 0);
  if (filled.length < words.length) {
    return { valid: false, error: null }; // incomplete, no error message yet
  }

  const phrase = words.map((w) => w.trim().toLowerCase()).join(" ");
  const isValid = validateMnemonic(phrase, wl);

  if (!isValid) {
    // Check which word is invalid
    for (let i = 0; i < words.length; i++) {
      const word = words[i]?.trim().toLowerCase() ?? "";
      if (!wl.includes(word)) {
        return {
          valid: false,
          error: `Word ${i + 1} ("${word}") is not a valid BIP39 word`,
        };
      }
    }
    return {
      valid: false,
      error: "Checksum verification failed. Please double-check your words.",
    };
  }

  return { valid: true, error: null };
}

// ─── Component ────────────────────────────────────────────────────

export function ImportWalletScreen() {
  const navigate = useNavigate();

  const [importMode, setImportMode] = useState<ImportMode>("mnemonic");
  const [importComplete, setImportComplete] = useState(false);
  const [importType, setImportType] = useState<"hd" | "singleKey">("hd");

  const handleBack = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const handleGoToWallets = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const handleCreateIdentity = useCallback(() => {
    navigate({ to: "/identities" as string });
  }, [navigate]);

  const handleImportAnother = useCallback(() => {
    setImportComplete(false);
    setImportType("hd");
  }, []);

  const handleSuccess = useCallback((type: "hd" | "singleKey") => {
    setImportType(type);
    setImportComplete(true);
  }, []);

  if (importComplete) {
    return (
      <Island className="max-w-2xl mx-auto">
        <ImportSuccessScreen
          importType={importType}
          onGoToWallets={handleGoToWallets}
          onCreateIdentity={handleCreateIdentity}
          onImportAnother={handleImportAnother}
        />
      </Island>
    );
  }

  return (
    <div className="flex flex-1 flex-col min-h-0 overflow-auto">
      <div className="flex items-center gap-3 mb-6">
        <Button
          variant="ghost"
          size="icon"
          onClick={handleBack}
          aria-label="Back to wallets"
        >
          <ArrowLeft className="h-5 w-5" />
        </Button>
        <h1 className="text-2xl font-bold">Import Wallet</h1>
      </div>

      <Island className="max-w-2xl">
        <Tabs
          value={importMode}
          onValueChange={(v) => setImportMode(v as ImportMode)}
        >
          <TabsList className="mb-6">
            <TabsTrigger value="mnemonic" className="gap-2">
              <Wallet className="h-4 w-4" />
              Seed Phrase
            </TabsTrigger>
            <TabsTrigger value="privateKey" className="gap-2">
              <Key className="h-4 w-4" />
              Private Key
            </TabsTrigger>
          </TabsList>

          <TabsContent value="mnemonic">
            <MnemonicImportForm onSuccess={() => handleSuccess("hd")} />
          </TabsContent>

          <TabsContent value="privateKey">
            <PrivateKeyImportForm
              onSuccess={() => handleSuccess("singleKey")}
            />
          </TabsContent>
        </Tabs>
      </Island>
    </div>
  );
}

// ─── Mnemonic Import Form ─────────────────────────────────────────

function MnemonicImportForm({ onSuccess }: { onSuccess: () => void }) {
  const [wordCount, setWordCount] = useState<WordCount>(24);
  const [words, setWords] = useState<string[]>(() => Array(24).fill(""));
  const [language, setLanguage] = useState<Bip39Language>("English");
  const [alias, setAlias] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [identityScanCount, setIdentityScanCount] = useState(10);

  const inputRefs = useRef<(HTMLInputElement | null)[]>([]);

  // Validation
  const validation = useMemo(
    () => validateSeedPhrase(words, WORDLISTS[language]),
    [words, language],
  );

  // Resize words array when word count changes
  useEffect(() => {
    setWords((prev) => {
      if (prev.length === wordCount) return prev;
      const next = Array(wordCount).fill("");
      for (let i = 0; i < Math.min(prev.length, wordCount); i++) {
        next[i] = prev[i];
      }
      return next;
    });
  }, [wordCount]);

  const handleWordChange = useCallback(
    (index: number, value: string) => {
      // Handle multi-word paste
      const pastedWords = value.trim().split(/\s+/);
      if (pastedWords.length > 1) {
        setWords((prev) => {
          const next = [...prev];
          for (
            let i = 0;
            i < pastedWords.length && index + i < next.length;
            i++
          ) {
            next[index + i] = pastedWords[i] ?? "";
          }
          return next;
        });
        // Focus the last filled input or the next empty one
        const lastIndex = Math.min(
          index + pastedWords.length - 1,
          wordCount - 1,
        );
        setTimeout(() => inputRefs.current[lastIndex]?.focus(), 0);
        return;
      }

      setWords((prev) => {
        const next = [...prev];
        next[index] = value;
        return next;
      });
    },
    [wordCount],
  );

  const handleSave = useCallback(async () => {
    if (!validation.valid) return;
    setSaving(true);

    try {
      const phrase = words.map((w) => w.trim().toLowerCase()).join(" ");
      const result = await commands.walletImportMnemonic({
        mnemonic: phrase,
        password,
        alias: alias.trim().slice(0, 64),
        usePasswordForApp: password.length > 0,
        identityScanCount: showAdvanced ? identityScanCount : 10,
      });

      if (result.status === "ok") {
        await useWalletStore.getState().loadWallets();
        toast.success("Wallet imported successfully");
        onSuccess();
      } else {
        toastError(result.error);
      }
    } catch (e) {
      toastError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [
    validation.valid,
    words,
    password,
    alias,
    showAdvanced,
    identityScanCount,
    onSuccess,
  ]);

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold mb-1">Import from Seed Phrase</h2>
        <p className="text-sm text-muted-foreground">
          Enter your BIP39 recovery seed phrase to import an existing HD wallet.
          You can paste the entire phrase into the first field.
        </p>
      </div>

      {/* Advanced options toggle */}
      <label className="flex items-center gap-3 cursor-pointer select-none">
        <input
          type="checkbox"
          checked={showAdvanced}
          onChange={(e) => setShowAdvanced(e.target.checked)}
          className="h-4 w-4 rounded border-input accent-primary"
        />
        <span className="text-sm font-medium">Show Advanced Options</span>
      </label>

      {/* Identity scan count */}
      {showAdvanced && (
        <div className="space-y-2">
          <Label htmlFor="identity-scan">Identity Auto-Discovery</Label>
          <div className="flex items-center gap-3">
            <Input
              id="identity-scan"
              type="number"
              min={0}
              max={50}
              value={identityScanCount}
              onChange={(e) => {
                const v = Math.min(50, Math.max(0, Number(e.target.value) || 0));
                setIdentityScanCount(v);
              }}
              className="w-[100px]"
            />
            <span className="text-xs text-muted-foreground">
              indices to scan (0 to disable)
            </span>
          </div>
        </div>
      )}

      {/* Language and word count selectors */}
      <div className="flex gap-4">
        <div className="space-y-2">
          <Label htmlFor="import-language">Language</Label>
          <Select
            value={language}
            onValueChange={(v) => setLanguage(v as Bip39Language)}
          >
            <SelectTrigger id="import-language" className="w-[180px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {BIP39_LANGUAGES.map((lang) => (
                <SelectItem key={lang} value={lang}>
                  {lang}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-2">
          <Label htmlFor="import-word-count">Word Count</Label>
          <Select
            value={String(wordCount)}
            onValueChange={(v) => setWordCount(Number(v) as WordCount)}
          >
            <SelectTrigger id="import-word-count" className="w-[180px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {WORD_COUNTS.map((count) => (
                <SelectItem key={count} value={String(count)}>
                  {count} words
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Word grid */}
      <div
        className="grid grid-cols-4 gap-2"
        role="group"
        aria-label="Seed phrase input"
      >
        {words.map((word, i) => (
          <div key={i} className="flex items-center gap-1">
            <span className="text-xs text-muted-foreground font-mono w-5 text-right shrink-0">
              {i + 1}.
            </span>
            <Input
              ref={(el) => {
                inputRefs.current[i] = el;
              }}
              value={word}
              onChange={(e) => handleWordChange(i, e.target.value)}
              placeholder={`Word ${i + 1}`}
              className={cn(
                "h-9 text-sm font-mono",
                validation.error &&
                  validation.error.includes(`Word ${i + 1}`) &&
                  "border-destructive",
              )}
              autoComplete="off"
              spellCheck={false}
              aria-label={`Word ${i + 1}`}
            />
          </div>
        ))}
      </div>

      {/* Validation error */}
      {validation.error && (
        <p className="text-sm text-destructive" role="alert">
          {validation.error}
        </p>
      )}

      {/* Gate: only show name/password if valid mnemonic */}
      {validation.valid && (
        <>
          <NameAndPasswordSection
            alias={alias}
            onAliasChange={setAlias}
            password={password}
            onPasswordChange={setPassword}
            showPassword={showPassword}
            onShowPasswordChange={setShowPassword}
            aliasPlaceholder="My Wallet"
          />

          <div className="flex justify-end pt-2">
            <Button onClick={handleSave} disabled={saving} className="gap-2">
              {saving ? (
                <>
                  <RefreshCw className="h-4 w-4 animate-spin" />
                  Importing...
                </>
              ) : (
                <>
                  <Wallet className="h-4 w-4" />
                  Import Wallet
                </>
              )}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

// ─── Private Key Import Form ──────────────────────────────────────

function PrivateKeyImportForm({ onSuccess }: { onSuccess: () => void }) {
  const [privateKey, setPrivateKey] = useState("");
  const [alias, setAlias] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [parseError, setParseError] = useState<string | null>(null);
  const [parsedAddress, setParsedAddress] = useState<string | null>(null);

  // Validate private key format on change
  useEffect(() => {
    const trimmed = privateKey.trim();
    if (!trimmed) {
      setParseError(null);
      setParsedAddress(null);
      return;
    }

    // Basic format validation
    const isWif =
      (trimmed.length === 51 || trimmed.length === 52) &&
      /^[5KL][1-9A-HJ-NP-Za-km-z]+$/.test(trimmed);
    const isHex = trimmed.length === 64 && /^[0-9a-fA-F]+$/.test(trimmed);

    if (!isWif && !isHex) {
      setParseError(
        "Invalid format. Enter a WIF key (51-52 chars) or hex key (64 chars).",
      );
      setParsedAddress(null);
    } else {
      setParseError(null);
      // We can't derive the address client-side without the backend,
      // so we'll show a format-valid indicator instead
      setParsedAddress(isWif ? "WIF format detected" : "Hex format detected");
    }
  }, [privateKey]);

  const handleSave = useCallback(async () => {
    const trimmed = privateKey.trim();
    if (!trimmed) return;
    setSaving(true);

    try {
      const result = await commands.walletImportPrivateKey({
        privateKey: trimmed,
        password,
        alias: alias.trim().slice(0, 64),
      });

      if (result.status === "ok") {
        await useWalletStore.getState().loadWallets();
        toast.success("Key imported successfully");
        onSuccess();
      } else {
        toastError(result.error);
      }
    } catch (e) {
      toastError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [privateKey, password, alias, onSuccess]);

  const isKeyValid =
    privateKey.trim().length > 0 && parseError === null;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold mb-1">Import Private Key</h2>
        <p className="text-sm text-muted-foreground">
          Enter a single private key to import as a simple wallet. Supports WIF
          format (51-52 characters) or raw hex (64 characters).
        </p>
      </div>

      {/* Private key input */}
      <div className="space-y-2">
        <Label htmlFor="private-key">Private Key</Label>
        <div className="relative">
          <Input
            id="private-key"
            type={showKey ? "text" : "password"}
            placeholder="Enter private key (WIF or hex)"
            value={privateKey}
            onChange={(e) => setPrivateKey(e.target.value)}
            className={cn("pr-10 font-mono", parseError && "border-destructive")}
            autoComplete="off"
            spellCheck={false}
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
            onClick={() => setShowKey(!showKey)}
            aria-label={showKey ? "Hide key" : "Show key"}
          >
            {showKey ? (
              <EyeOff className="h-4 w-4 text-muted-foreground" />
            ) : (
              <Eye className="h-4 w-4 text-muted-foreground" />
            )}
          </Button>
        </div>

        {parseError && (
          <p className="text-sm text-destructive" role="alert">
            {parseError}
          </p>
        )}

        {parsedAddress && !parseError && (
          <p className="text-sm text-success">{parsedAddress}</p>
        )}
      </div>

      {/* Gate: only show name/password if key looks valid */}
      {isKeyValid && (
        <>
          <NameAndPasswordSection
            alias={alias}
            onAliasChange={setAlias}
            password={password}
            onPasswordChange={setPassword}
            showPassword={showPassword}
            onShowPasswordChange={setShowPassword}
            aliasPlaceholder="My Key"
          />

          <div className="flex justify-end pt-2">
            <Button onClick={handleSave} disabled={saving} className="gap-2">
              {saving ? (
                <>
                  <RefreshCw className="h-4 w-4 animate-spin" />
                  Importing...
                </>
              ) : (
                <>
                  <Key className="h-4 w-4" />
                  Import Key
                </>
              )}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

// ─── Shared Name & Password Section ───────────────────────────────

function NameAndPasswordSection({
  alias,
  onAliasChange,
  password,
  onPasswordChange,
  showPassword,
  onShowPasswordChange,
  aliasPlaceholder,
}: {
  alias: string;
  onAliasChange: (value: string) => void;
  password: string;
  onPasswordChange: (value: string) => void;
  showPassword: boolean;
  onShowPasswordChange: (show: boolean) => void;
  aliasPlaceholder: string;
}) {
  return (
    <>
      {/* Name */}
      <div className="space-y-2">
        <Label htmlFor="import-alias">Name</Label>
        <Input
          id="import-alias"
          placeholder={aliasPlaceholder}
          value={alias}
          onChange={(e) => onAliasChange(e.target.value.slice(0, 64))}
          maxLength={64}
        />
        <div className="flex justify-between">
          <p className="text-xs text-muted-foreground">
            Leave empty for auto-generated name
          </p>
          {alias.length > 50 && (
            <span className="text-xs text-muted-foreground">
              {alias.length}/64
            </span>
          )}
        </div>
      </div>

      {/* Password */}
      <div className="space-y-2">
        <Label htmlFor="import-password">Password (Optional)</Label>
        <div className="relative">
          <Input
            id="import-password"
            type={showPassword ? "text" : "password"}
            placeholder="Enter password to encrypt"
            value={password}
            onChange={(e) => onPasswordChange(e.target.value)}
            className="pr-10"
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
            onClick={() => onShowPasswordChange(!showPassword)}
            aria-label={showPassword ? "Hide password" : "Show password"}
          >
            {showPassword ? (
              <EyeOff className="h-4 w-4 text-muted-foreground" />
            ) : (
              <Eye className="h-4 w-4 text-muted-foreground" />
            )}
          </Button>
        </div>

        <PasswordStrengthMeter password={password} />
      </div>
    </>
  );
}

// ─── Import Success Screen ────────────────────────────────────────

function ImportSuccessScreen({
  importType,
  onGoToWallets,
  onCreateIdentity,
  onImportAnother,
}: {
  importType: "hd" | "singleKey";
  onGoToWallets: () => void;
  onCreateIdentity: () => void;
  onImportAnother: () => void;
}) {
  const isHd = importType === "hd";
  const title = isHd
    ? "Wallet Imported Successfully!"
    : "Key Imported Successfully!";

  return (
    <div className="flex flex-col items-center text-center py-8 space-y-6">
      <div className="flex items-center justify-center w-16 h-16 rounded-full bg-success/20">
        <Check className="h-8 w-8 text-success" />
      </div>

      <div>
        <h2 className="text-2xl font-bold">{title}</h2>
        <p className="text-muted-foreground mt-2">
          {isHd
            ? "Your wallet has been imported and is ready to use."
            : "Your private key wallet has been imported."}
        </p>
      </div>

      <div className="flex flex-col gap-3 w-full max-w-md">
        <Button onClick={onGoToWallets} className="w-full">
          Go to Wallet
        </Button>
        {isHd && (
          <Button
            onClick={onCreateIdentity}
            variant="outline"
            className="w-full"
          >
            Create Identity
          </Button>
        )}
        <Button
          onClick={onImportAnother}
          variant="ghost"
          className="w-full"
        >
          Import Another
        </Button>
      </div>
    </div>
  );
}
