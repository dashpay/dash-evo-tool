import { useCallback, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowLeft,
  Check,
  Eye,
  EyeOff,
  RefreshCw,
  Wallet,
} from "lucide-react";
import { entropyToMnemonic } from "@scure/bip39";
import { wordlist as englishWordlist } from "@scure/bip39/wordlists/english.js";
import { wordlist as spanishWordlist } from "@scure/bip39/wordlists/spanish.js";
import { wordlist as frenchWordlist } from "@scure/bip39/wordlists/french.js";
import { wordlist as italianWordlist } from "@scure/bip39/wordlists/italian.js";
import { wordlist as portugueseWordlist } from "@scure/bip39/wordlists/portuguese.js";
import { toast } from "sonner";
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
import { CopyButton } from "@/components/shared/CopyButton";
import { commands } from "@/bindings";
import { cn } from "@/lib/utils";
import { useWalletStore } from "@/stores/walletStore";
import {
  EntropyGrid,
  type EntropyGridRef,
} from "@/components/wallet/EntropyGrid";

// ─── Constants ────────────────────────────────────────────────────

const WORD_COUNTS = [12, 15, 18, 21, 24] as const;
type WordCount = (typeof WORD_COUNTS)[number];

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

const ENTROPY_BITS: Record<WordCount, number> = {
  12: 128,
  15: 160,
  18: 192,
  21: 224,
  24: 256,
};

const STRENGTH_LABELS = ["Very Weak", "Weak", "Fair", "Strong", "Very Strong"];
const STRENGTH_COLORS = [
  "bg-destructive",
  "bg-destructive",
  "bg-warning",
  "bg-success",
  "bg-success",
];

type Step = "generate" | "backup" | "protect" | "success";

// ─── Helpers ──────────────────────────────────────────────────────

function estimatePasswordStrength(password: string): number {
  if (!password) return 0;
  let score = 0;
  if (password.length >= 8) score++;
  if (password.length >= 12) score++;
  if (/[a-z]/.test(password) && /[A-Z]/.test(password)) score++;
  if (/\d/.test(password)) score++;
  if (/[^a-zA-Z0-9]/.test(password)) score++;
  return Math.min(4, score);
}

// ─── Component ────────────────────────────────────────────────────

export function CreateWalletScreen() {
  const navigate = useNavigate();

  const [step, setStep] = useState<Step>("generate");
  const [wordCount, setWordCount] = useState<WordCount>(24);
  const [language, setLanguage] = useState<Bip39Language>("English");
  const [mnemonic, setMnemonic] = useState<string[] | null>(null);
  const [wroteItDown, setWroteItDown] = useState(false);
  const [alias, setAlias] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [saving, setSaving] = useState(false);
  const [createdSeedHash, setCreatedSeedHash] = useState<string | null>(null);
  const entropyGridRef = useRef<EntropyGridRef>(null);

  const passwordStrength = useMemo(
    () => estimatePasswordStrength(password),
    [password],
  );

  const handleGenerate = useCallback(() => {
    const entropyBytes = ENTROPY_BITS[wordCount] / 8;
    // Get combined entropy: user-modified grid XORed with fresh WebCrypto randomness
    const fullEntropy = entropyGridRef.current?.getCombinedEntropy();
    const entropy = fullEntropy
      ? fullEntropy.slice(0, entropyBytes)
      : crypto.getRandomValues(new Uint8Array(entropyBytes));
    const mnemonicStr = entropyToMnemonic(entropy, WORDLISTS[language]);
    setMnemonic(mnemonicStr.split(" "));
    setStep("backup");
    setWroteItDown(false);
  }, [wordCount, language]);

  const handleSave = useCallback(async () => {
    if (!mnemonic) return;
    setSaving(true);

    try {
      const result = await commands.walletCreate({
        mnemonic: mnemonic.join(" "),
        password,
        alias: alias.trim().slice(0, 64),
        usePasswordForApp: password.length > 0,
      });

      if (result.status === "ok") {
        setCreatedSeedHash(result.data.seedHash);
        await useWalletStore.getState().loadWallets();
        setStep("success");
      } else {
        toast.error(result.error);
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [mnemonic, alias, password]);

  const handleBack = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const handleGoToWallets = useCallback(() => {
    navigate({ to: "/wallets" });
  }, [navigate]);

  const handleCreateIdentity = useCallback(() => {
    navigate({ to: "/identities" as string });
  }, [navigate]);

  // ─── Success ────────────────────────────────────────────────────

  if (step === "success") {
    return (
      <Island className="max-w-2xl mx-auto">
        <SuccessScreen
          onGoToWallets={handleGoToWallets}
          onCreateIdentity={handleCreateIdentity}
          seedHash={createdSeedHash}
        />
      </Island>
    );
  }

  // ─── Wizard ─────────────────────────────────────────────────────

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
        <h1 className="text-2xl font-bold">Create New Wallet</h1>
      </div>

      <div
        className="flex items-center gap-2 mb-8"
        role="navigation"
        aria-label="Wallet creation steps"
      >
        <StepIndicator
          step={1}
          label="Generate"
          active={step === "generate"}
          completed={step !== "generate"}
        />
        <StepConnector completed={step !== "generate"} />
        <StepIndicator
          step={2}
          label="Backup"
          active={step === "backup"}
          completed={step === "protect" || step === "success"}
        />
        <StepConnector completed={step === "protect" || step === "success"} />
        <StepIndicator
          step={3}
          label="Protect"
          active={step === "protect"}
          completed={step === "success"}
        />
      </div>

      <Island className="max-w-2xl">
        {step === "generate" && (
          <GenerateStep
            wordCount={wordCount}
            onWordCountChange={setWordCount}
            language={language}
            onLanguageChange={setLanguage}
            onGenerate={handleGenerate}
            entropyGridRef={entropyGridRef}
          />
        )}

        {step === "backup" && mnemonic && (
          <BackupStep
            words={mnemonic}
            wroteItDown={wroteItDown}
            onWroteItDownChange={setWroteItDown}
            onBack={() => {
              setStep("generate");
              setMnemonic(null);
              setWroteItDown(false);
            }}
            onNext={() => setStep("protect")}
          />
        )}

        {step === "protect" && (
          <ProtectStep
            alias={alias}
            onAliasChange={setAlias}
            password={password}
            onPasswordChange={setPassword}
            showPassword={showPassword}
            onShowPasswordChange={setShowPassword}
            passwordStrength={passwordStrength}
            saving={saving}
            onBack={() => setStep("backup")}
            onSave={handleSave}
          />
        )}
      </Island>
    </div>
  );
}

// ─── Sub-components ─────────────────────────────────────────────────

function StepIndicator({
  step,
  label,
  active,
  completed,
}: {
  step: number;
  label: string;
  active: boolean;
  completed: boolean;
}) {
  return (
    <div
      className="flex items-center gap-2"
      aria-current={active ? "step" : undefined}
    >
      <div
        className={cn(
          "flex items-center justify-center w-8 h-8 rounded-full text-sm font-medium transition-colors",
          active && "bg-primary text-primary-foreground",
          completed && "bg-primary/20 text-primary",
          !active && !completed && "bg-muted text-muted-foreground",
        )}
      >
        {completed ? <Check className="h-4 w-4" /> : step}
      </div>
      <span
        className={cn(
          "text-sm font-medium",
          active && "text-foreground",
          !active && "text-muted-foreground",
        )}
      >
        {label}
      </span>
    </div>
  );
}

function StepConnector({ completed }: { completed: boolean }) {
  return (
    <div
      className={cn(
        "flex-1 h-px max-w-[60px]",
        completed ? "bg-primary/40" : "bg-border",
      )}
    />
  );
}

function GenerateStep({
  wordCount,
  onWordCountChange,
  language,
  onLanguageChange,
  onGenerate,
  entropyGridRef,
}: {
  wordCount: WordCount;
  onWordCountChange: (count: WordCount) => void;
  language: Bip39Language;
  onLanguageChange: (lang: Bip39Language) => void;
  onGenerate: () => void;
  entropyGridRef: React.RefObject<EntropyGridRef | null>;
}) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold mb-1">Generate Seed Phrase</h2>
        <p className="text-sm text-muted-foreground">
          Move your cursor over the grid below to contribute extra randomness,
          then configure and generate your seed phrase.
        </p>
      </div>

      <EntropyGrid ref={entropyGridRef} />

      <div className="flex gap-4">
        <div className="space-y-2">
          <Label htmlFor="language">Language</Label>
          <Select
            value={language}
            onValueChange={(v) => onLanguageChange(v as Bip39Language)}
          >
            <SelectTrigger id="language" className="w-[180px]">
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
          <Label htmlFor="word-count">Word Count</Label>
          <Select
            value={String(wordCount)}
            onValueChange={(v) => onWordCountChange(Number(v) as WordCount)}
          >
            <SelectTrigger id="word-count" className="w-[180px]">
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

      <Button onClick={onGenerate} className="gap-2">
        <RefreshCw className="h-4 w-4" />
        Generate Seed Phrase
      </Button>
    </div>
  );
}

function BackupStep({
  words,
  wroteItDown,
  onWroteItDownChange,
  onBack,
  onNext,
}: {
  words: string[];
  wroteItDown: boolean;
  onWroteItDownChange: (checked: boolean) => void;
  onBack: () => void;
  onNext: () => void;
}) {
  const cols = words.length <= 12 ? 3 : 4;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold mb-1">Back Up Your Seed Phrase</h2>
        <p className="text-sm text-muted-foreground">
          Write down these words in order on paper. Keep them in a safe place.
          Never share your seed phrase with anyone.
        </p>
      </div>

      <div
        className={cn(
          "grid gap-2",
          cols === 3 ? "grid-cols-3" : "grid-cols-4",
        )}
        role="list"
        aria-label="Seed phrase words"
      >
        {words.map((word, i) => (
          <div
            key={i}
            className="flex items-center gap-2 rounded-md border bg-muted/50 px-3 py-2"
            role="listitem"
          >
            <span className="text-xs text-muted-foreground font-mono w-5 text-right">
              {i + 1}.
            </span>
            <span className="text-sm font-medium font-mono">{word}</span>
          </div>
        ))}
      </div>

      <div className="flex items-center gap-2">
        <CopyButton
          value={words.join(" ")}
          label="Copy All Words"
        />
        <span className="text-xs text-muted-foreground">
          (For pasting into another backup method)
        </span>
      </div>

      <label className="flex items-center gap-3 cursor-pointer select-none">
        <input
          type="checkbox"
          checked={wroteItDown}
          onChange={(e) => onWroteItDownChange(e.target.checked)}
          className="h-4 w-4 rounded border-input accent-primary"
        />
        <span className="text-sm font-medium">
          I have written down my seed phrase and stored it securely
        </span>
      </label>

      <div className="flex justify-between pt-2">
        <Button variant="outline" onClick={onBack}>
          Back
        </Button>
        <Button onClick={onNext} disabled={!wroteItDown}>
          Continue
        </Button>
      </div>
    </div>
  );
}

function ProtectStep({
  alias,
  onAliasChange,
  password,
  onPasswordChange,
  showPassword,
  onShowPasswordChange,
  passwordStrength,
  saving,
  onBack,
  onSave,
}: {
  alias: string;
  onAliasChange: (value: string) => void;
  password: string;
  onPasswordChange: (value: string) => void;
  showPassword: boolean;
  onShowPasswordChange: (show: boolean) => void;
  passwordStrength: number;
  saving: boolean;
  onBack: () => void;
  onSave: () => void;
}) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold mb-1">Name & Protect</h2>
        <p className="text-sm text-muted-foreground">
          Give your wallet a name and optionally protect it with a password.
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="wallet-alias">Wallet Name</Label>
        <Input
          id="wallet-alias"
          placeholder="My Wallet"
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

      <div className="space-y-2">
        <Label htmlFor="wallet-password">Password (Optional)</Label>
        <div className="relative">
          <Input
            id="wallet-password"
            type={showPassword ? "text" : "password"}
            placeholder="Enter password to encrypt wallet"
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

        {password.length > 0 && (
          <div className="space-y-1">
            <div className="flex gap-1">
              {[0, 1, 2, 3, 4].map((i) => (
                <div
                  key={i}
                  className={cn(
                    "h-1.5 flex-1 rounded-full transition-colors",
                    i <= passwordStrength
                      ? STRENGTH_COLORS[passwordStrength]
                      : "bg-muted",
                  )}
                />
              ))}
            </div>
            <p className="text-xs text-muted-foreground">
              Strength: {STRENGTH_LABELS[passwordStrength]}
            </p>
          </div>
        )}
      </div>

      <div className="flex justify-between pt-2">
        <Button variant="outline" onClick={onBack} disabled={saving}>
          Back
        </Button>
        <Button onClick={onSave} disabled={saving} className="gap-2">
          {saving ? (
            <>
              <RefreshCw className="h-4 w-4 animate-spin" />
              Creating...
            </>
          ) : (
            <>
              <Wallet className="h-4 w-4" />
              Create Wallet
            </>
          )}
        </Button>
      </div>
    </div>
  );
}

function SuccessScreen({
  onGoToWallets,
  onCreateIdentity,
  seedHash: _seedHash,
}: {
  onGoToWallets: () => void;
  onCreateIdentity: () => void;
  seedHash: string | null;
}) {
  return (
    <div className="flex flex-col items-center text-center py-8 space-y-6">
      <div className="flex items-center justify-center w-16 h-16 rounded-full bg-success/20">
        <Check className="h-8 w-8 text-success" />
      </div>

      <div>
        <h2 className="text-2xl font-bold">Wallet Created Successfully!</h2>
        <p className="text-muted-foreground mt-2">
          Your new wallet is ready to use. Here are some next steps:
        </p>
      </div>

      <div className="space-y-3 text-left w-full max-w-md">
        <div className="flex items-start gap-3 rounded-lg border p-4">
          <div className="flex items-center justify-center w-6 h-6 rounded-full bg-primary/20 text-primary text-xs font-bold shrink-0 mt-0.5">
            1
          </div>
          <div>
            <p className="text-sm font-medium">Fund your wallet</p>
            <p className="text-xs text-muted-foreground">
              Send DASH to your wallet to get started. You can find your receive
              address in the wallet detail view.
            </p>
          </div>
        </div>
        <div className="flex items-start gap-3 rounded-lg border p-4">
          <div className="flex items-center justify-center w-6 h-6 rounded-full bg-primary/20 text-primary text-xs font-bold shrink-0 mt-0.5">
            2
          </div>
          <div>
            <p className="text-sm font-medium">Create a Platform Identity</p>
            <p className="text-xs text-muted-foreground">
              Register an identity on Dash Platform to use DashPay, DPNS names,
              and other features.
            </p>
          </div>
        </div>
      </div>

      <div className="flex flex-col sm:flex-row gap-3 w-full max-w-md">
        <Button onClick={onGoToWallets} className="flex-1">
          Go to Wallet
        </Button>
        <Button onClick={onCreateIdentity} variant="outline" className="flex-1">
          Create Identity
        </Button>
      </div>
    </div>
  );
}
