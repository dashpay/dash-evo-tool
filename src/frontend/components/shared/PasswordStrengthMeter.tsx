import { useMemo } from "react";
import zxcvbn from "zxcvbn";
import { cn } from "@/lib/utils";

const STRENGTH_LABELS = ["Very Weak", "Weak", "Fair", "Strong", "Very Strong"];
const STRENGTH_COLORS = [
  "bg-destructive",
  "bg-destructive",
  "bg-warning",
  "bg-success",
  "bg-success",
];

interface PasswordStrengthMeterProps {
  password: string;
}

export interface PasswordStrengthResult {
  score: number;
  crackTime: string;
}

export function usePasswordStrength(password: string): PasswordStrengthResult {
  return useMemo(() => {
    if (!password) return { score: 0, crackTime: "" };
    const result = zxcvbn(password);
    const crackTimeDisplay = String(
      result.crack_times_display.offline_slow_hashing_1e4_per_second,
    );
    return {
      score: result.score,
      crackTime:
        crackTimeDisplay === "less than a second"
          ? "<1 second"
          : crackTimeDisplay,
    };
  }, [password]);
}

export function PasswordStrengthMeter({ password }: PasswordStrengthMeterProps) {
  const { score, crackTime } = usePasswordStrength(password);

  if (!password) return null;

  return (
    <div className="space-y-1">
      <div className="flex gap-1">
        {[0, 1, 2, 3, 4].map((i) => (
          <div
            key={i}
            className={cn(
              "h-1.5 flex-1 rounded-full transition-colors",
              i <= score ? STRENGTH_COLORS[score] : "bg-muted",
            )}
          />
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        Strength: {STRENGTH_LABELS[score]}
      </p>
      {crackTime && (
        <p className="text-xs text-muted-foreground">
          Estimated time to crack: {crackTime}
        </p>
      )}
    </div>
  );
}
