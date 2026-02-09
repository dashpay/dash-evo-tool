import { render, screen, fireEvent } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { createRef } from "react";
import {
  EntropyGrid,
  type EntropyGridRef,
  getBit,
  toggleBit,
  xorBytes,
} from "./EntropyGrid";

// Mock crypto.getRandomValues for deterministic tests
const mockGetRandomValues = vi.fn((arr: Uint8Array) => {
  for (let i = 0; i < arr.length; i++) {
    arr[i] = i; // Deterministic pattern: 0, 1, 2, ...
  }
  return arr;
});

beforeEach(() => {
  vi.stubGlobal("crypto", { getRandomValues: mockGetRandomValues });
});

// ─── Helper Unit Tests ───────────────────────────────────────────

describe("getBit", () => {
  it("returns false for zero byte", () => {
    const bytes = new Uint8Array(32);
    expect(getBit(bytes, 0)).toBe(false);
    expect(getBit(bytes, 255)).toBe(false);
  });

  it("returns true for set bit", () => {
    const bytes = new Uint8Array(32);
    bytes[0] = 0b00000001; // bit 0 set
    expect(getBit(bytes, 0)).toBe(true);
    expect(getBit(bytes, 1)).toBe(false);
  });

  it("reads correct bit from byte 1", () => {
    const bytes = new Uint8Array(32);
    bytes[1] = 0b00000100; // bit 10 (byte 1, bit 2)
    expect(getBit(bytes, 10)).toBe(true);
    expect(getBit(bytes, 8)).toBe(false);
    expect(getBit(bytes, 9)).toBe(false);
  });
});

describe("toggleBit", () => {
  it("toggles bit 0 on", () => {
    const bytes = new Uint8Array(32);
    const result = toggleBit(bytes, 0);
    expect(getBit(result, 0)).toBe(true);
    // Original unchanged
    expect(getBit(bytes, 0)).toBe(false);
  });

  it("toggles bit 0 off", () => {
    const bytes = new Uint8Array(32);
    bytes[0] = 0b00000001;
    const result = toggleBit(bytes, 0);
    expect(getBit(result, 0)).toBe(false);
  });

  it("toggles bit 255", () => {
    const bytes = new Uint8Array(32);
    const result = toggleBit(bytes, 255);
    expect(getBit(result, 255)).toBe(true);
    // Byte 31, bit 7
    expect(result[31]).toBe(0b10000000);
  });
});

describe("xorBytes", () => {
  it("XORs two arrays", () => {
    const a = new Uint8Array(32);
    const b = new Uint8Array(32);
    a[0] = 0xff;
    b[0] = 0x0f;
    const result = xorBytes(a, b);
    expect(result[0]).toBe(0xf0);
  });

  it("XOR with self produces zeros", () => {
    const a = new Uint8Array(32);
    a.fill(0xab);
    const result = xorBytes(a, a);
    expect(result.every((b) => b === 0)).toBe(true);
  });

  it("XOR with zeros is identity", () => {
    const a = new Uint8Array(32);
    a.fill(0x42);
    const zeros = new Uint8Array(32);
    const result = xorBytes(a, zeros);
    expect(Array.from(result)).toEqual(Array.from(a));
  });
});

// ─── Component Tests ─────────────────────────────────────────────

describe("EntropyGrid", () => {
  it("renders 256 grid cells", () => {
    render(<EntropyGrid />);
    const grid = screen.getByRole("grid", { name: /entropy grid/i });
    expect(grid).toBeInTheDocument();
    const cells = screen.getAllByRole("gridcell");
    expect(cells.length).toBe(256);
  });

  it("shows instruction text", () => {
    render(<EntropyGrid />);
    expect(
      screen.getByText(/click and drag across the grid to add randomness/i),
    ).toBeInTheDocument();
  });

  it("shows bits flipped counter starting at 0", () => {
    render(<EntropyGrid />);
    expect(screen.getByText(/0 bits flipped/i)).toBeInTheDocument();
  });

  it("toggles a bit on pointer down", () => {
    render(<EntropyGrid />);
    const cell0 = screen.getByLabelText("bit 0");
    const initialOn = cell0.dataset.on;

    fireEvent.pointerDown(cell0);

    // After toggle, the data-on should have flipped
    expect(cell0.dataset.on).not.toBe(initialOn);
    // Bits flipped counter increments
    expect(screen.getByText(/1 bit flipped/i)).toBeInTheDocument();
  });

  it("toggles multiple bits on drag", () => {
    render(<EntropyGrid />);
    const cell0 = screen.getByLabelText("bit 0");
    const cell1 = screen.getByLabelText("bit 1");
    const cell2 = screen.getByLabelText("bit 2");

    fireEvent.pointerDown(cell0);
    fireEvent.pointerEnter(cell1);
    fireEvent.pointerEnter(cell2);

    expect(screen.getByText(/3 bits flipped/i)).toBeInTheDocument();
  });

  it("does not toggle on hover without pointer down", () => {
    render(<EntropyGrid />);
    const cell1 = screen.getByLabelText("bit 1");

    fireEvent.pointerEnter(cell1);

    expect(screen.getByText(/0 bits flipped/i)).toBeInTheDocument();
  });

  it("stops toggling after pointer up", () => {
    render(<EntropyGrid />);
    const cell0 = screen.getByLabelText("bit 0");
    const cell1 = screen.getByLabelText("bit 1");

    fireEvent.pointerDown(cell0);
    fireEvent(window, new Event("pointerup"));
    fireEvent.pointerEnter(cell1);

    expect(screen.getByText(/1 bit flipped/i)).toBeInTheDocument();
  });

  it("does not toggle same bit twice in one drag", () => {
    render(<EntropyGrid />);
    const cell0 = screen.getByLabelText("bit 0");

    fireEvent.pointerDown(cell0);
    // Entering same cell again should not re-toggle
    fireEvent.pointerEnter(cell0);

    expect(screen.getByText(/1 bit flipped/i)).toBeInTheDocument();
  });

  it("shows 'Entropy locked' text when frozen", () => {
    render(<EntropyGrid frozen />);
    expect(screen.getByText("Entropy locked")).toBeInTheDocument();
  });

  it("does not toggle when frozen", () => {
    render(<EntropyGrid frozen />);
    const cell0 = screen.getByLabelText("bit 0");
    const initialOn = cell0.dataset.on;

    fireEvent.pointerDown(cell0);

    expect(cell0.dataset.on).toBe(initialOn);
    expect(screen.getByText(/0 bits flipped/i)).toBeInTheDocument();
  });

  it("exposes getCombinedEntropy via ref", () => {
    const ref = createRef<EntropyGridRef>();
    render(<EntropyGrid ref={ref} />);

    expect(ref.current).not.toBeNull();
    const entropy = ref.current!.getCombinedEntropy();
    expect(entropy).toBeInstanceOf(Uint8Array);
    expect(entropy.length).toBe(32);
  });

  it("getCombinedEntropy XORs grid state with fresh randomness", () => {
    const ref = createRef<EntropyGridRef>();
    render(<EntropyGrid ref={ref} />);

    // The initial grid state is randomBytes() which returns [0,1,2,...,31]
    // getCombinedEntropy calls xorBytes(gridState, randomBytes())
    // Second call to randomBytes() also returns [0,1,2,...,31] due to mock
    // So XOR of identical arrays = all zeros
    const entropy = ref.current!.getCombinedEntropy();
    expect(entropy.every((b) => b === 0)).toBe(true);
  });

  it("has aria-readonly when frozen", () => {
    render(<EntropyGrid frozen />);
    const grid = screen.getByRole("grid", { name: /entropy grid/i });
    expect(grid).toHaveAttribute("aria-readonly", "true");
  });
});
