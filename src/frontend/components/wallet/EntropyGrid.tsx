import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { cn } from "@/lib/utils";

// ─── Constants ────────────────────────────────────────────────────

const ROWS = 8;
const COLS = 32;
const TOTAL_BITS = ROWS * COLS; // 256

// ─── Helpers ──────────────────────────────────────────────────────

/** Generate 32 bytes of cryptographic randomness. */
function randomBytes(): Uint8Array {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return bytes;
}

/** Get the value of bit at `position` (0-255) within a 32-byte array. */
function getBit(bytes: Uint8Array, position: number): boolean {
  const byteIndex = Math.floor(position / 8);
  const bitIndex = position % 8;
  return ((bytes[byteIndex] >> bitIndex) & 1) === 1;
}

/** Toggle bit at `position` (0-255) within a 32-byte array (XOR). */
function toggleBit(bytes: Uint8Array, position: number): Uint8Array {
  const result = new Uint8Array(bytes);
  const byteIndex = Math.floor(position / 8);
  const bitIndex = position % 8;
  result[byteIndex] ^= 1 << bitIndex;
  return result;
}

/** XOR two 32-byte arrays together. */
function xorBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const result = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    result[i] = a[i] ^ b[i];
  }
  return result;
}

// ─── Export helpers for testing ───────────────────────────────────
export { getBit, toggleBit, xorBytes, randomBytes };

// ─── Types ────────────────────────────────────────────────────────

export interface EntropyGridRef {
  /** Returns user entropy XORed with fresh WebCrypto randomness. */
  getCombinedEntropy: () => Uint8Array;
}

export interface EntropyGridProps {
  /** When true, the grid becomes non-interactive (after generation). */
  frozen?: boolean;
  /** Custom class name. */
  className?: string;
}

// ─── Component ────────────────────────────────────────────────────

export const EntropyGrid = forwardRef<EntropyGridRef, EntropyGridProps>(
  function EntropyGrid({ frozen = false, className }, ref) {
    const [bytes, setBytes] = useState<Uint8Array>(() => randomBytes());
    const [bitsToggled, setBitsToggled] = useState(0);
    const lastBitRef = useRef<number>(-1);
    const isPointerDownRef = useRef(false);
    const bytesRef = useRef(bytes);

    // Keep ref in sync with state
    useEffect(() => {
      bytesRef.current = bytes;
    }, [bytes]);

    // Expose getCombinedEntropy to parent via ref
    useImperativeHandle(
      ref,
      () => ({
        getCombinedEntropy: () => xorBytes(bytesRef.current, randomBytes()),
      }),
      [],
    );

    const handleToggle = useCallback(
      (position: number) => {
        if (frozen) return;
        if (position === lastBitRef.current) return;
        lastBitRef.current = position;
        setBytes((prev) => toggleBit(prev, position));
        setBitsToggled((prev) => prev + 1);
      },
      [frozen],
    );

    const handlePointerDown = useCallback(
      (position: number) => {
        if (frozen) return;
        isPointerDownRef.current = true;
        lastBitRef.current = -1; // Reset so this position can toggle
        handleToggle(position);
      },
      [frozen, handleToggle],
    );

    const handlePointerEnter = useCallback(
      (position: number) => {
        if (frozen || !isPointerDownRef.current) return;
        handleToggle(position);
      },
      [frozen, handleToggle],
    );

    // Listen for pointer up globally so we catch releases outside the grid
    useEffect(() => {
      const handleUp = () => {
        isPointerDownRef.current = false;
        lastBitRef.current = -1;
      };
      window.addEventListener("pointerup", handleUp);
      return () => window.removeEventListener("pointerup", handleUp);
    }, []);

    return (
      <div className={cn("space-y-2", className)}>
        <div className="flex items-center justify-between">
          <p className="text-xs text-muted-foreground select-none">
            {frozen
              ? "Entropy locked"
              : "Click and drag across the grid to add randomness"}
          </p>
          <span className="text-xs text-muted-foreground font-mono tabular-nums">
            {bitsToggled} bit{bitsToggled !== 1 ? "s" : ""} flipped
          </span>
        </div>
        <div
          role="grid"
          aria-label="Entropy grid"
          aria-readonly={frozen}
          className={cn(
            "inline-grid select-none touch-none rounded border bg-muted/30 p-px gap-0",
            frozen && "opacity-60 pointer-events-none",
          )}
          style={{
            gridTemplateColumns: `repeat(${COLS}, 1fr)`,
            gridTemplateRows: `repeat(${ROWS}, 1fr)`,
            maxWidth: 400,
            width: "100%",
          }}
        >
          {Array.from({ length: TOTAL_BITS }, (_, i) => {
            const isOn = getBit(bytes, i);
            return (
              <div
                key={i}
                role="gridcell"
                aria-label={`bit ${i}`}
                data-bit={i}
                data-on={isOn}
                className={cn(
                  "aspect-square transition-colors duration-75",
                  isOn
                    ? "bg-[--dash-blue] dark:bg-[--dash-blue]/85"
                    : "bg-background dark:bg-[hsl(0,0%,31%)]",
                  !frozen && "cursor-pointer hover:opacity-80",
                )}
                onPointerDown={(e) => {
                  e.preventDefault();
                  handlePointerDown(i);
                }}
                onPointerEnter={() => handlePointerEnter(i)}
              />
            );
          })}
        </div>
      </div>
    );
  },
);
