import { useCallback, useMemo } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

/** Minimal identity info for the selector. */
export interface IdentityOption {
  /** Hex-encoded identity ID */
  id: string;
  /** Display label (alias or truncated ID) */
  displayName: string;
}

interface IdentitySelectorProps {
  /** Currently selected identity ID (hex string) or empty for "Other" */
  value: string;
  /** Called when the selection changes */
  onChange: (identityId: string) => void;
  /** Available identities to choose from */
  identities: IdentityOption[];
  /** Identity IDs to exclude from the dropdown */
  exclude?: string[];
  /** Optional label */
  label?: string;
  /** Show "Other" option for manual ID entry (default: true) */
  showOther?: boolean;
  /** Placeholder for the dropdown when nothing is selected */
  placeholder?: string;
  /** Additional CSS class */
  className?: string;
  /** Disable the selector */
  disabled?: boolean;
}

const OTHER_VALUE = "__other__";

/**
 * Combined dropdown + text input for selecting blockchain identities.
 *
 * Matches egui IdentitySelector behavior:
 * - Dropdown with known identities
 * - "Other" option for manual ID entry
 * - Auto-sync between dropdown and text field
 * - Exclude list to hide specific identities
 */
export function IdentitySelector({
  value,
  onChange,
  identities,
  exclude = [],
  label,
  showOther = true,
  placeholder = "Select identity",
  className,
  disabled = false,
}: IdentitySelectorProps) {
  const excludeSet = useMemo(() => new Set(exclude), [exclude]);

  const filteredIdentities = useMemo(
    () => identities.filter((id) => !excludeSet.has(id.id)),
    [identities, excludeSet],
  );

  // Determine if the current value matches a known identity
  const isKnownIdentity = useMemo(
    () => filteredIdentities.some((id) => id.id === value),
    [filteredIdentities, value],
  );

  // The select value: either the identity ID or "__other__"
  const selectValue = isKnownIdentity ? value : value ? OTHER_VALUE : "";

  const handleSelectChange = useCallback(
    (newValue: string) => {
      if (newValue === OTHER_VALUE) {
        onChange("");
      } else {
        onChange(newValue);
      }
    },
    [onChange],
  );

  const handleTextChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange(e.target.value);
    },
    [onChange],
  );

  const showTextInput = showOther && !isKnownIdentity;

  return (
    <div className={cn("space-y-1.5", className)}>
      {label && (
        <Label className="text-sm font-medium">{label}</Label>
      )}
      <div className="flex items-center gap-2">
        <Select
          value={selectValue}
          onValueChange={handleSelectChange}
          disabled={disabled}
        >
          <SelectTrigger className="w-[200px]" aria-label={label || "Identity"}>
            <SelectValue placeholder={placeholder} />
          </SelectTrigger>
          <SelectContent>
            {filteredIdentities.map((identity) => (
              <SelectItem key={identity.id} value={identity.id}>
                {identity.displayName}
              </SelectItem>
            ))}
            {showOther && filteredIdentities.length > 0 && (
              <SelectSeparator />
            )}
            {showOther && (
              <SelectItem value={OTHER_VALUE}>Other</SelectItem>
            )}
            {filteredIdentities.length === 0 && !showOther && (
              <SelectItem value="__empty__" disabled>
                No identities found
              </SelectItem>
            )}
          </SelectContent>
        </Select>

        {showTextInput && (
          <Input
            type="text"
            placeholder="Enter identity ID"
            value={value}
            onChange={handleTextChange}
            disabled={disabled}
            className="flex-1"
            aria-label="Identity ID"
          />
        )}
      </div>
    </div>
  );
}
