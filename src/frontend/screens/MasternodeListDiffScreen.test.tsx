import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { MasternodeListDiffScreen } from "./MasternodeListDiffScreen";

// ─── Centralized mock bindings ──────────────────────────────────

const { mocks, mockNavigate } = await vi.hoisted(async () => {
  const { createMockBindings } = await import("../test/mock-ipc");
  const initial = createMockBindings();
  const mockNavigate = vi.fn();
  return { mocks: initial, mockNavigate };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/bindings", () => ({
  commands: mocks.commands,
  events: mocks.events,
}));

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

// ─── Helpers ────────────────────────────────────────────────────────────────

/** Flush microtask queue so all async event listeners are registered. */
async function flushMicrotasks() {
  await act(async () => {});
}

function emitChainLockedBlock(payload: Record<string, unknown>) {
  act(() => {
    mocks.emitMockEvent("zmqChainLockedBlockEvent", payload);
  });
}

function emitIsLockedTransaction(payload: Record<string, unknown>) {
  act(() => {
    mocks.emitMockEvent("zmqIsLockedTransactionEvent", payload);
  });
}

function createBlockPayload(
  height: number,
  overrides: Record<string, unknown> = {},
) {
  return {
    network: "Testnet",
    blockHeight: height,
    blockHash: `00000000000000000000000000000000000000000000000000000000000${height.toString(16).padStart(5, "0")}`,
    txCount: 2,
    txIds: [`tx1-block-${height}`, `tx2-block-${height}`],
    rawBlock: `rawblock${height}`,
    rawChainLock: `rawlock${height}`,
    signature: `sig${height}`,
    isValid: true,
    ...overrides,
  };
}

function createTxPayload(
  txid: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    network: "Testnet",
    txid,
    rawTx: `rawtx-${txid}`,
    rawIsLock: `rawislock-${txid}`,
    affectedUtxoCount: 1,
    isValid: true,
    ...overrides,
  };
}

// ─── Tests ──────────────────────────────────────────────────────────────────

describe("MasternodeListDiffScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Clear event listeners between tests
    for (const arr of mocks.eventListeners.values()) {
      arr.length = 0;
    }
  });

  // ─── Rendering ──────────────────────────────────────────────────────────

  it("renders the screen title and tabs", () => {
    render(<MasternodeListDiffScreen />);
    expect(screen.getByText("Masternode List Diff")).toBeInTheDocument();
    expect(screen.getByText("Core Items")).toBeInTheDocument();
    expect(screen.getByText("QR Info")).toBeInTheDocument();
    expect(screen.getByText("Quorum Viewer")).toBeInTheDocument();
  });

  it("renders the Core Items tab active by default", () => {
    render(<MasternodeListDiffScreen />);
    expect(screen.getByTestId("core-items-tab")).toBeInTheDocument();
  });

  it("shows empty state for blocks and transactions", () => {
    render(<MasternodeListDiffScreen />);
    expect(
      screen.getByText("Waiting for chain-locked blocks…"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Waiting for IS-locked transactions…"),
    ).toBeInTheDocument();
  });

  it("shows placeholder text when no item is selected", () => {
    render(<MasternodeListDiffScreen />);
    expect(
      screen.getByText("Select an item to view details."),
    ).toBeInTheDocument();
  });

  it("renders back button to tools page", () => {
    render(<MasternodeListDiffScreen />);
    expect(screen.getByLabelText("Back to Tools")).toBeInTheDocument();
  });

  it("disables QR Info and Quorum Viewer tabs", () => {
    render(<MasternodeListDiffScreen />);
    const qrInfoTab = screen.getByText("QR Info").closest("button");
    const quorumTab = screen.getByText("Quorum Viewer").closest("button");
    expect(qrInfoTab).toBeDisabled();
    expect(quorumTab).toBeDisabled();
  });

  // ─── ZMQ Chain Locked Block Events ──────────────────────────────────────

  it("displays a chain-locked block when ZMQ event arrives", () => {
    render(<MasternodeListDiffScreen />);
    emitChainLockedBlock(createBlockPayload(12345));
    expect(screen.getByText("12345")).toBeInTheDocument();
  });

  it("displays multiple blocks sorted by height descending", () => {
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(createBlockPayload(100));
    emitChainLockedBlock(createBlockPayload(200));

    const blockList = screen.getByRole("listbox", {
      name: "ChainLocked Blocks",
    });
    const options = within(blockList).getAllByRole("option");
    expect(options).toHaveLength(2);
    // Higher block first
    expect(options[0]).toHaveTextContent("200");
    expect(options[1]).toHaveTextContent("100");
  });

  it("shows block count badge", () => {
    render(<MasternodeListDiffScreen />);
    emitChainLockedBlock(createBlockPayload(100));
    emitChainLockedBlock(createBlockPayload(200));

    // Badge with count "2" — locate in the blocks section
    const blockSection = screen
      .getByText("ChainLocked Blocks")
      .closest("div")!;
    expect(within(blockSection).getByText("2")).toBeInTheDocument();
  });

  it("shows valid checkmark for valid block", () => {
    render(<MasternodeListDiffScreen />);
    emitChainLockedBlock(createBlockPayload(100, { isValid: true }));

    const blockList = screen.getByRole("listbox", {
      name: "ChainLocked Blocks",
    });
    const validIcons = within(blockList).getAllByLabelText("Valid");
    expect(validIcons).toHaveLength(1);
  });

  it("shows invalid icon for invalid block", () => {
    render(<MasternodeListDiffScreen />);
    emitChainLockedBlock(createBlockPayload(100, { isValid: false }));

    const blockList = screen.getByRole("listbox", {
      name: "ChainLocked Blocks",
    });
    const invalidIcons = within(blockList).getAllByLabelText("Invalid");
    expect(invalidIcons).toHaveLength(1);
  });

  // ─── ZMQ InstantSend Transaction Events ─────────────────────────────────

  it("displays an instant send transaction when ZMQ event arrives", async () => {
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();
    emitIsLockedTransaction(createTxPayload("deadbeef1234567890abcdef"));
    expect(screen.getByText(/TxID: deadbeef/)).toBeInTheDocument();
  });

  it("shows transaction count badge", async () => {
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();
    emitIsLockedTransaction(createTxPayload("tx1"));
    emitIsLockedTransaction(createTxPayload("tx2"));
    emitIsLockedTransaction(createTxPayload("tx3"));

    const txSection = screen
      .getByText("Instant Send Transactions")
      .closest("div")!;
    expect(within(txSection).getByText("3")).toBeInTheDocument();
  });

  // ─── Selection & Details ────────────────────────────────────────────────

  it("shows chain lock details when a block is selected", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(
      createBlockPayload(12345, {
        txIds: ["tx-abc", "tx-def"],
        signature: "signatureHex123",
      }),
    );

    await user.click(screen.getByText("12345"));

    expect(screen.getByTestId("chain-lock-details")).toBeInTheDocument();
    expect(screen.getByText("ChainLock Details")).toBeInTheDocument();
    expect(screen.getByText("signatureHex123")).toBeInTheDocument();

    // Block transactions
    expect(screen.getByText("Block Transactions (2)")).toBeInTheDocument();
    expect(screen.getByText("tx-abc")).toBeInTheDocument();
    expect(screen.getByText("tx-def")).toBeInTheDocument();
  });

  it("shows instant send details when a transaction is selected", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();

    emitIsLockedTransaction(
      createTxPayload("abcdef1234567890", {
        rawTx: "0100000001abcdef",
        rawIsLock: "instantlock0123",
      }),
    );

    await user.click(screen.getByText(/TxID:/));

    expect(screen.getByTestId("instant-send-details")).toBeInTheDocument();
    expect(screen.getByText("Instant Send Details")).toBeInTheDocument();
    expect(screen.getByText("abcdef1234567890")).toBeInTheDocument();
    expect(screen.getByText("0100000001abcdef")).toBeInTheDocument();
    expect(screen.getByText("instantlock0123")).toBeInTheDocument();
  });

  it("switches selection between block and transaction", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();

    emitChainLockedBlock(createBlockPayload(500));
    emitIsLockedTransaction(createTxPayload("tx-switch-test"));

    // Select block
    await user.click(screen.getByText("500"));
    expect(screen.getByTestId("chain-lock-details")).toBeInTheDocument();

    // Select transaction
    await user.click(screen.getByText(/TxID:/));
    expect(screen.getByTestId("instant-send-details")).toBeInTheDocument();
    expect(screen.queryByTestId("chain-lock-details")).not.toBeInTheDocument();
  });

  it("highlights the selected block", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(createBlockPayload(100));
    emitChainLockedBlock(createBlockPayload(200));

    const blockList = screen.getByRole("listbox", {
      name: "ChainLocked Blocks",
    });
    const options = within(blockList).getAllByRole("option");

    // Select height 100 (index 1 due to descending sort)
    await user.click(options[1]);
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    expect(options[0]).toHaveAttribute("aria-selected", "false");
  });

  // ─── Raw Data Display ───────────────────────────────────────────────────

  it("shows raw block and chain lock data in details", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(
      createBlockPayload(100, {
        rawBlock: "rawblockdata123",
        rawChainLock: "rawlockdata456",
      }),
    );

    await user.click(screen.getByText("100"));

    expect(screen.getByText("rawblockdata123")).toBeInTheDocument();
    expect(screen.getByText("rawlockdata456")).toBeInTheDocument();
  });

  it("shows raw transaction and instant lock data in details", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();

    emitIsLockedTransaction(
      createTxPayload("mytx", {
        rawTx: "rawtxdata789",
        rawIsLock: "rawislockdataabc",
      }),
    );

    await user.click(screen.getByText(/TxID:/));

    expect(screen.getByText("rawtxdata789")).toBeInTheDocument();
    expect(screen.getByText("rawislockdataabc")).toBeInTheDocument();
  });

  // ─── Empty block transactions ───────────────────────────────────────────

  it("shows no-transactions message for empty block", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(createBlockPayload(100, { txIds: [], txCount: 0 }));

    await user.click(screen.getByText("100"));

    expect(
      screen.getByText("No transactions in this block."),
    ).toBeInTheDocument();
  });

  // ─── Validation display in details ──────────────────────────────────────

  it("shows valid status in chain lock details", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(createBlockPayload(100, { isValid: true }));
    await user.click(screen.getByText("100"));

    const details = screen.getByTestId("chain-lock-details");
    expect(within(details).getByText("Yes")).toBeInTheDocument();
  });

  it("shows invalid status in chain lock details", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(createBlockPayload(100, { isValid: false }));
    await user.click(screen.getByText("100"));

    const details = screen.getByTestId("chain-lock-details");
    expect(within(details).getByText("No")).toBeInTheDocument();
  });

  it("shows valid status in instant send details", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();

    emitIsLockedTransaction(createTxPayload("tx1", { isValid: true }));
    await user.click(screen.getByText(/TxID:/));

    const details = screen.getByTestId("instant-send-details");
    expect(within(details).getByText("Yes")).toBeInTheDocument();
  });

  it("shows invalid status in instant send details", async () => {
    const user = userEvent.setup();
    render(<MasternodeListDiffScreen />);
    await flushMicrotasks();

    emitIsLockedTransaction(createTxPayload("tx1", { isValid: false }));
    await user.click(screen.getByText(/TxID:/));

    const details = screen.getByTestId("instant-send-details");
    expect(within(details).getByText("No")).toBeInTheDocument();
  });

  // ─── Block deduplication ────────────────────────────────────────────────

  it("replaces block data when same height arrives again", () => {
    render(<MasternodeListDiffScreen />);

    emitChainLockedBlock(createBlockPayload(100, { signature: "old-sig" }));
    emitChainLockedBlock(createBlockPayload(100, { signature: "new-sig" }));

    const blockList = screen.getByRole("listbox", {
      name: "ChainLocked Blocks",
    });
    // Should only have 1 entry, not 2
    const options = within(blockList).getAllByRole("option");
    expect(options).toHaveLength(1);
  });
});
