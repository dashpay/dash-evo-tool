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
    expect(screen.getByText("Diffs")).toBeInTheDocument();
    expect(screen.getByText("Chain Lock Sigs")).toBeInTheDocument();
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

  it("enables all three tabs", () => {
    render(<MasternodeListDiffScreen />);
    const qrInfoTab = screen.getByText("QR Info").closest("button");
    const quorumTab = screen.getByText("Quorum Viewer").closest("button");
    expect(qrInfoTab).not.toBeDisabled();
    expect(quorumTab).not.toBeDisabled();
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

  // ─── QR Info Tab ──────────────────────────────────────────────────────

  describe("QR Info Tab", () => {
    const mockQrInfo = {
      tipBlockHash: "000000000000000000000000000000000000000000000000000000000000aaaa",
      snapshotHMinusC: { skipListMode: 0, activeQuorumMembers: [true, false, true], skipList: [10, 20] },
      snapshotHMinus2C: { skipListMode: 1, activeQuorumMembers: [false, true], skipList: [] },
      snapshotHMinus3C: { skipListMode: 2, activeQuorumMembers: [true], skipList: [5] },
      snapshotHMinus4C: null,
      diffHMinus3C: createMockDiff("diff-h3c"),
      diffHMinus2C: createMockDiff("diff-h2c"),
      diffHMinusC: createMockDiff("diff-hc"),
      diffH: createMockDiff("diff-h"),
      diffTip: createMockDiff("diff-tip"),
      diffHMinus4C: null,
      lastCommitments: [createMockQuorumEntry(0), createMockQuorumEntry(1)],
      quorumSnapshotList: [
        { skipListMode: 0, activeQuorumMembers: [true, true], skipList: [1] },
      ],
      mnListDiffList: [createMockDiff("list-0")],
    };

    function createMockDiff(id: string) {
      return {
        version: 1,
        baseBlockHash: `basehash-${id}`,
        blockHash: `blockhash-${id}`,
        totalTransactions: 5,
        merkleHashes: [`merkle1-${id}`],
        merkleFlagsLen: 2,
        coinbaseTxid: `cbtxid-${id}`,
        coinbaseSize: 200,
        newMasternodes: [{ proRegTxHash: `mn-${id}`, address: "1.2.3.4:9999" }],
        deletedMasternodes: [],
        newQuorums: [],
        deletedQuorums: [],
        chainlockSigCount: 0,
        chainlockSignatures: [],
      };
    }

    function createMockQuorumEntry(index: number) {
      return {
        version: 2,
        llmqType: 4,
        quorumHash: `qhash-${index}`,
        quorumIndex: index,
        signers: [true, false, true, false, true, false, true, false],
        validMembers: [true, true, false, false, true, true, false, false],
        quorumPublicKey: `pubkey-${index}`,
        quorumVvecHash: `vvechash-${index}`,
        thresholdSig: `threshsig-${index}`,
        allCommitmentAggregatedSignature: `aggsig-${index}`,
      };
    }

    async function switchToQrInfoTab() {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
    }

    it("renders QR Info tab with load button", async () => {
      await switchToQrInfoTab();
      expect(screen.getByTestId("qr-info-tab")).toBeInTheDocument();
      expect(screen.getByTestId("load-qrinfo-button")).toBeInTheDocument();
    });

    it("shows empty state before loading a file", async () => {
      await switchToQrInfoTab();
      expect(screen.getByText(/Load a QRInfo .dat file/)).toBeInTheDocument();
    });

    it("displays QRInfo field list after loading", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));

      // Wait for the data to be loaded
      await screen.findByText("Quorum Snapshots");
      expect(screen.getByText("Masternode List Diffs")).toBeInTheDocument();
      expect(screen.getByText("Rotated Quorums")).toBeInTheDocument();
      expect(screen.getByText("Quorum Snapshot List")).toBeInTheDocument();
      expect(screen.getByText("MN List Diff List")).toBeInTheDocument();
    });

    it("shows tip block hash after loading", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));

      await screen.findByText(/Tip:/);
    });

    it("shows snapshot items when Quorum Snapshots field is selected", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Quorum Snapshots");

      await user.click(screen.getByText("Quorum Snapshots"));
      expect(screen.getByText("Snapshot h-c")).toBeInTheDocument();
      expect(screen.getByText("Snapshot h-2c")).toBeInTheDocument();
      expect(screen.getByText("Snapshot h-3c")).toBeInTheDocument();
      // h-4c should not be shown since it's null
      expect(screen.queryByText("Snapshot h-4c")).not.toBeInTheDocument();
    });

    it("shows snapshot detail when a snapshot item is clicked", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Quorum Snapshots");

      await user.click(screen.getByText("Quorum Snapshots"));
      await user.click(screen.getByText("Snapshot h-c"));

      expect(screen.getByTestId("snapshot-detail")).toBeInTheDocument();
      expect(screen.getByText("Quorum Snapshot")).toBeInTheDocument();
      // Check active members count (2 of 3 are active)
      expect(screen.getByText(/Active Quorum Members \(2\/3\)/)).toBeInTheDocument();
    });

    it("shows diff items when Masternode List Diffs field is selected", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Masternode List Diffs");

      await user.click(screen.getByText("Masternode List Diffs"));
      expect(screen.getByText("MNListDiff h-3c")).toBeInTheDocument();
      expect(screen.getByText("MNListDiff h-2c")).toBeInTheDocument();
      expect(screen.getByText("MNListDiff h-c")).toBeInTheDocument();
      expect(screen.getByText("MNListDiff h")).toBeInTheDocument();
      expect(screen.getByText("MNListDiff Tip")).toBeInTheDocument();
    });

    it("shows diff detail when a diff item is clicked", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Masternode List Diffs");

      await user.click(screen.getByText("Masternode List Diffs"));
      await user.click(screen.getByText("MNListDiff h-3c"));

      expect(screen.getByTestId("mnlistdiff-detail")).toBeInTheDocument();
      expect(screen.getByText("MnListDiff")).toBeInTheDocument();
      expect(screen.getByText("basehash-diff-h3c")).toBeInTheDocument();
    });

    it("shows quorum entry items when Rotated Quorums field is selected", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Rotated Quorums");

      await user.click(screen.getByText("Rotated Quorums"));
      expect(screen.getByText("Quorum at Index 0")).toBeInTheDocument();
      expect(screen.getByText("Quorum at Index 1")).toBeInTheDocument();
    });

    it("shows quorum entry detail with members grid", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Rotated Quorums");

      await user.click(screen.getByText("Rotated Quorums"));
      await user.click(screen.getByText("Quorum at Index 0"));

      expect(screen.getByTestId("quorum-entry-detail")).toBeInTheDocument();
      expect(screen.getByTestId("members-grid")).toBeInTheDocument();
      // Check signers/valid counts: 4 signers out of 8, 4 valid out of 8
      const counts = screen.getAllByText("4 / 8");
      expect(counts.length).toBe(2); // Total Signers + Valid Members
    });

    it("shows error when file load fails", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "error",
        error: "Failed to decode QRInfo file: invalid data",
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));

      await screen.findByText(/Failed to decode/);
    });

    it("does not show error when file selection is cancelled", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "error",
        error: "File selection cancelled",
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));

      // Wait a tick then verify no error shown
      await act(async () => {});
      expect(screen.queryByText(/File selection cancelled/)).not.toBeInTheDocument();
    });

    it("clears selection when switching fields", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Quorum Snapshots");

      // Select a snapshot
      await user.click(screen.getByText("Quorum Snapshots"));
      await user.click(screen.getByText("Snapshot h-c"));
      expect(screen.getByTestId("snapshot-detail")).toBeInTheDocument();

      // Switch to a different field
      await user.click(screen.getByText("Rotated Quorums"));
      expect(screen.queryByTestId("snapshot-detail")).not.toBeInTheDocument();
      expect(screen.getByText("Select an item to view details.")).toBeInTheDocument();
    });

    it("shows Quorum Snapshot List items", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("Quorum Snapshot List");

      await user.click(screen.getByText("Quorum Snapshot List"));
      expect(screen.getByText("Snapshot 0")).toBeInTheDocument();
    });

    it("shows MN List Diff List items", async () => {
      mocks.commands.qrinfoLoadFile = vi.fn().mockResolvedValue({
        status: "ok",
        data: mockQrInfo,
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("QR Info"));
      await user.click(screen.getByTestId("load-qrinfo-button"));
      await screen.findByText("MN List Diff List");

      await user.click(screen.getByText("MN List Diff List"));
      expect(screen.getByText("MNListDiff 0")).toBeInTheDocument();
    });
  });

  // ─── Input Area ─────────────────────────────────────────────────────────────

  describe("Input Area", () => {
    it("renders base height and end height inputs", () => {
      render(<MasternodeListDiffScreen />);
      expect(screen.getByTestId("base-height-input")).toBeInTheDocument();
      expect(screen.getByTestId("end-height-input")).toBeInTheDocument();
    });

    it("renders all action buttons", () => {
      render(<MasternodeListDiffScreen />);
      expect(screen.getByTestId("fetch-diff-button")).toBeInTheDocument();
      expect(screen.getByTestId("fetch-qrinfo-button")).toBeInTheDocument();
      expect(screen.getByTestId("fetch-dmls-no-rotation-button")).toBeInTheDocument();
      expect(screen.getByTestId("fetch-dmls-with-rotation-button")).toBeInTheDocument();
      expect(screen.getByTestId("fetch-chain-locks-button")).toBeInTheDocument();
      expect(screen.getByTestId("clear-button")).toBeInTheDocument();
      expect(screen.getByTestId("clear-keep-base-button")).toBeInTheDocument();
    });

    it("shows error for invalid base height", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      const baseInput = screen.getByTestId("base-height-input");
      await user.type(baseInput, "abc");
      await user.click(screen.getByTestId("fetch-diff-button"));
      await screen.findByText("Invalid base block height");
    });

    it("shows clear success message when Clear is clicked", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByTestId("clear-button"));
      await screen.findByText("Cleared all data");
    });

    it("dismisses message when X is clicked", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByTestId("clear-button"));
      await screen.findByText("Cleared all data");
      await user.click(screen.getByLabelText("Dismiss message"));
      expect(screen.queryByText("Cleared all data")).not.toBeInTheDocument();
    });

    it("calls mnlistFetchDiff when fetch diff button is clicked", async () => {
      mocks.commands.mnlistFetchDiff = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-1" },
      });
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      const baseInput = screen.getByTestId("base-height-input");
      const endInput = screen.getByTestId("end-height-input");
      await user.type(baseInput, "100");
      await user.type(endInput, "200");
      await user.click(screen.getByTestId("fetch-diff-button"));
      expect(mocks.commands.mnlistFetchDiff).toHaveBeenCalledWith(
        expect.objectContaining({
          baseBlockHeight: 100,
          blockHeight: 200,
          validateQuorums: false,
        }),
      );
    });

    it("calls mnlistFetchChainLocks when chain locks button is clicked", async () => {
      mocks.commands.mnlistFetchChainLocks = vi.fn().mockResolvedValue({
        taskId: "task-2",
      });
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      const baseInput = screen.getByTestId("base-height-input");
      const endInput = screen.getByTestId("end-height-input");
      await user.type(baseInput, "50");
      await user.type(endInput, "100");
      await user.click(screen.getByTestId("fetch-chain-locks-button"));
      expect(mocks.commands.mnlistFetchChainLocks).toHaveBeenCalledWith(
        expect.objectContaining({
          baseBlockHeight: 50,
          blockHeight: 100,
        }),
      );
    });

    it("shows pending indicator during fetch", async () => {
      mocks.commands.mnlistFetchDiff = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-3" },
      });
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByTestId("fetch-diff-button"));
      expect(screen.getByTestId("pending-indicator")).toBeInTheDocument();
      expect(screen.getByText("Fetching DML diff…")).toBeInTheDocument();
    });

    it("disables buttons while pending", async () => {
      mocks.commands.mnlistFetchDiff = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-4" },
      });
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByTestId("fetch-diff-button"));
      expect(screen.getByTestId("fetch-qrinfo-button")).toBeDisabled();
      expect(screen.getByTestId("fetch-chain-locks-button")).toBeDisabled();
    });
  });

  // ─── Quorum Viewer Tab ──────────────────────────────────────────────────────

  describe("Quorum Viewer Tab", () => {
    it("shows empty state when no quorum data is available", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("Quorum Viewer"));
      expect(screen.getByTestId("quorum-viewer-tab")).toBeInTheDocument();
      expect(screen.getByText("No quorum data available.")).toBeInTheDocument();
    });

    it("shows quorum data from task result events", async () => {
      // First setup a fetch diff command
      mocks.commands.mnlistFetchDiff = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-10" },
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await flushMicrotasks();

      // Trigger a fetch
      await user.click(screen.getByTestId("fetch-diff-button"));

      // Simulate task result with diff containing quorums
      act(() => {
        mocks.emitMockEvent("taskResultEvent", {
          taskId: "task-10",
          result: {
            type: "mnListFetchedDiff",
            baseHeight: 0,
            height: 100,
            diff: {
              version: 1,
              baseBlockHash: "0".repeat(64),
              blockHash: "1".repeat(64),
              totalTransactions: 1,
              merkleHashes: [],
              merkleFlagsLen: 0,
              coinbaseTxid: "a".repeat(64),
              coinbaseSize: 100,
              newMasternodes: [],
              deletedMasternodes: [],
              newQuorums: [
                {
                  version: 1,
                  llmqType: 4,
                  quorumHash: "abcd".repeat(16),
                  quorumIndex: null,
                  signers: [true, true, false, false],
                  validMembers: [true, true, true, false],
                  quorumPublicKey: "pk123",
                  quorumVvecHash: "vvec123",
                  thresholdSig: "sig123",
                  allCommitmentAggregatedSignature: "agg123",
                },
              ],
              deletedQuorums: [],
              chainlockSigCount: 0,
              chainlockSignatures: [],
            },
          },
        });
      });

      // Navigate to Quorum Viewer tab
      await user.click(screen.getByText("Quorum Viewer"));
      expect(screen.getByTestId("quorum-viewer-tab")).toBeInTheDocument();

      // Should show the LLMQ type button
      expect(screen.getByTestId("quorum-type-4")).toBeInTheDocument();
      expect(screen.getByText("LLMQ_100_67")).toBeInTheDocument();
    });

    it("shows quorum details when a quorum hash is selected", async () => {
      mocks.commands.mnlistFetchDiff = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-11" },
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await flushMicrotasks();

      await user.click(screen.getByTestId("fetch-diff-button"));

      // Simulate task result
      act(() => {
        mocks.emitMockEvent("taskResultEvent", {
          taskId: "task-11",
          result: {
            type: "mnListFetchedDiff",
            baseHeight: 0,
            height: 200,
            diff: {
              version: 1,
              baseBlockHash: "0".repeat(64),
              blockHash: "2".repeat(64),
              totalTransactions: 1,
              merkleHashes: [],
              merkleFlagsLen: 0,
              coinbaseTxid: "b".repeat(64),
              coinbaseSize: 100,
              newMasternodes: [],
              deletedMasternodes: [],
              newQuorums: [
                {
                  version: 1,
                  llmqType: 6,
                  quorumHash: "ef01".repeat(16),
                  quorumIndex: 2,
                  signers: [true, true, true, false, false],
                  validMembers: [true, true, false, false, false],
                  quorumPublicKey: "pubkey-test-1234",
                  quorumVvecHash: "vvec456",
                  thresholdSig: "tsig456",
                  allCommitmentAggregatedSignature: "agg456",
                },
              ],
              deletedQuorums: [],
              chainlockSigCount: 0,
              chainlockSignatures: [],
            },
          },
        });
      });

      // Navigate to Quorum Viewer tab
      await user.click(screen.getByText("Quorum Viewer"));

      // Click on the quorum type
      await user.click(screen.getByTestId("quorum-type-6"));

      // Click on the quorum hash entry
      const quorumList = screen.getByRole("listbox", { name: "Quorum Hashes" });
      const options = within(quorumList).getAllByRole("option");
      expect(options).toHaveLength(1);
      await user.click(options[0]);

      // Check detail panel shows the details
      expect(screen.getByText("Quorum Details")).toBeInTheDocument();
      expect(screen.getByText("pubkey-test-1234")).toBeInTheDocument();
      expect(screen.getByText("3 / 5")).toBeInTheDocument(); // signers
      expect(screen.getByText("2 / 5")).toBeInTheDocument(); // valid
      expect(screen.getByText("Height: 200")).toBeInTheDocument();
    });
  });

  // ─── Chain Lock Sigs Tab ──────────────────────────────────────────────────

  describe("Chain Lock Sigs Tab", () => {
    it("shows empty state when no chain lock sig data", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("Chain Lock Sigs"));
      expect(screen.getByTestId("chain-lock-sigs-tab")).toBeInTheDocument();
      expect(screen.getByText("No chain lock signature data available.")).toBeInTheDocument();
    });

    it("shows chain lock sigs from task result events", async () => {
      mocks.commands.mnlistFetchChainLocks = vi.fn().mockResolvedValue({
        taskId: "task-cl-1",
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await flushMicrotasks();

      await user.click(screen.getByTestId("fetch-chain-locks-button"));

      act(() => {
        mocks.emitMockEvent("taskResultEvent", {
          taskId: "task-cl-1",
          result: {
            type: "mnListChainLockSigs",
            entries: [
              { height: 100, blockHash: "a".repeat(64), signature: "sig100" },
              { height: 200, blockHash: "b".repeat(64), signature: null },
            ],
          },
        });
      });

      await user.click(screen.getByText("Chain Lock Sigs"));
      expect(screen.getByTestId("chain-lock-sigs-tab")).toBeInTheDocument();
      expect(screen.getByText("100")).toBeInTheDocument();
      expect(screen.getByText("200")).toBeInTheDocument();
      expect(screen.getByText("None")).toBeInTheDocument();
    });
  });

  // ─── Diffs Tab ────────────────────────────────────────────────────────────

  describe("Diffs Tab", () => {
    function createMockDiffResult(taskId: string, baseHeight: number, height: number) {
      return {
        taskId,
        result: {
          type: "mnListFetchedDiff",
          baseHeight,
          height,
          diff: {
            version: 1,
            baseBlockHash: "0".repeat(64),
            blockHash: "1".repeat(64),
            totalTransactions: 5,
            merkleHashes: [],
            merkleFlagsLen: 0,
            coinbaseTxid: "c".repeat(64),
            coinbaseSize: 100,
            newMasternodes: [
              { proRegTxHash: "mn-hash-aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aabb7788", address: "1.2.3.4:9999" },
              { proRegTxHash: "mn-hash-xxxx", address: "5.6.7.8:19999" },
            ],
            deletedMasternodes: ["deleted-mn-hash-1111222233334444555566667777888899990000aaaabbbbccccdddd"],
            newQuorums: [
              {
                version: 1,
                llmqType: 4,
                quorumHash: "qhash123".padEnd(64, "0"),
                quorumIndex: null,
                signers: [true, false],
                validMembers: [true, true],
                quorumPublicKey: "pk-test",
                quorumVvecHash: "vvec-test",
                thresholdSig: "tsig-test",
                allCommitmentAggregatedSignature: "agg-test",
              },
            ],
            deletedQuorums: [],
            chainlockSigCount: 1,
            chainlockSignatures: [{ signature: "clsig-test-1234", indexSet: [0, 1] }],
          },
        },
      };
    }

    async function setupWithDiff() {
      mocks.commands.mnlistFetchDiff = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-d1" },
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await flushMicrotasks();

      await user.click(screen.getByTestId("fetch-diff-button"));

      act(() => {
        mocks.emitMockEvent("taskResultEvent", createMockDiffResult("task-d1", 0, 100));
      });

      return user;
    }

    it("shows empty state when no diffs are fetched", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByText("Diffs"));
      expect(screen.getByTestId("diffs-tab")).toBeInTheDocument();
      expect(screen.getByText("No diff data available.")).toBeInTheDocument();
    });

    it("shows diff list when diffs are fetched", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      const options = within(diffList).getAllByRole("option");
      expect(options).toHaveLength(1);
      expect(options[0]).toHaveTextContent("0 → 100");
    });

    it("shows new quorums when a diff is selected", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));

      // Default sub-view is "New Quorums"
      expect(screen.getByText("LLMQ_100_67")).toBeInTheDocument();
    });

    it("shows masternode changes when MN Changes sub-view is selected", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));

      await user.click(screen.getByText("MN Changes"));
      expect(screen.getByText("New Masternodes (2)")).toBeInTheDocument();
      expect(screen.getByText("Deleted Masternodes (1)")).toBeInTheDocument();
      expect(screen.getByText("1.2.3.4:9999")).toBeInTheDocument();
    });

    it("shows chain lock signatures when Chain Locks sub-view is selected", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));

      await user.click(screen.getByText("Chain Locks"));
      expect(screen.getByText(/Index set: \[0, 1\]/)).toBeInTheDocument();
    });

    it("shows quorum detail when a quorum is selected in New Quorums sub-view", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));

      // Click on the quorum entry in the items list
      const itemList = screen.getByRole("listbox", { name: "Diff Items" });
      await user.click(within(itemList).getByRole("option"));

      expect(screen.getByTestId("quorum-entry-detail")).toBeInTheDocument();
      expect(screen.getByText("pk-test")).toBeInTheDocument();
    });

    it("shows masternode detail when a masternode is selected", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));
      await user.click(screen.getByText("MN Changes"));

      // Click on first new masternode
      const itemList = screen.getByRole("listbox", { name: "Diff Items" });
      const items = within(itemList).getAllByRole("option");
      await user.click(items[0]);

      const detail = screen.getByTestId("masternode-detail");
      expect(detail).toBeInTheDocument();
      expect(within(detail).getByText("New Masternode")).toBeInTheDocument();
      expect(within(detail).getByText("1.2.3.4:9999")).toBeInTheDocument();
    });

    it("filters masternodes with search (3+ chars)", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));
      await user.click(screen.getByText("MN Changes"));

      // Initially shows 2 new + 1 deleted
      expect(screen.getByText("New Masternodes (2)")).toBeInTheDocument();

      const searchInput = screen.getByTestId("mn-search-input");
      await user.type(searchInput, "xxxx");

      // Should filter to only the "xxxx" masternode
      expect(screen.getByText("New Masternodes (1)")).toBeInTheDocument();
      expect(screen.getByText("5.6.7.8:19999")).toBeInTheDocument();
    });

    it("shows no results message when search has no matches", async () => {
      const user = await setupWithDiff();
      await user.click(screen.getByText("Diffs"));

      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      await user.click(within(diffList).getByRole("option"));
      await user.click(screen.getByText("MN Changes"));

      const searchInput = screen.getByTestId("mn-search-input");
      await user.type(searchInput, "zzzzzz");

      expect(screen.getByText("No masternodes match your search.")).toBeInTheDocument();
    });
  });

  // ─── Clear Keep Base ──────────────────────────────────────────────────────

  describe("Clear Keep Base", () => {
    it("keeps only the base diff (baseHeight === 0) when clearKeepBase is clicked", async () => {
      mocks.commands.mnlistFetchDiff = vi.fn()
        .mockResolvedValueOnce({ status: "ok", data: { taskId: "task-ckb1" } })
        .mockResolvedValueOnce({ status: "ok", data: { taskId: "task-ckb2" } });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await flushMicrotasks();

      // Fetch two diffs
      await user.click(screen.getByTestId("fetch-diff-button"));
      act(() => {
        mocks.emitMockEvent("taskResultEvent", {
          taskId: "task-ckb1",
          result: {
            type: "mnListFetchedDiff",
            baseHeight: 0,
            height: 100,
            diff: {
              version: 1, baseBlockHash: "0".repeat(64), blockHash: "1".repeat(64),
              totalTransactions: 1, merkleHashes: [], merkleFlagsLen: 0,
              coinbaseTxid: "a".repeat(64), coinbaseSize: 100,
              newMasternodes: [], deletedMasternodes: [],
              newQuorums: [], deletedQuorums: [],
              chainlockSigCount: 0, chainlockSignatures: [],
            },
          },
        });
      });

      await user.click(screen.getByTestId("fetch-diff-button"));
      act(() => {
        mocks.emitMockEvent("taskResultEvent", {
          taskId: "task-ckb2",
          result: {
            type: "mnListFetchedDiff",
            baseHeight: 100,
            height: 200,
            diff: {
              version: 1, baseBlockHash: "1".repeat(64), blockHash: "2".repeat(64),
              totalTransactions: 1, merkleHashes: [], merkleFlagsLen: 0,
              coinbaseTxid: "b".repeat(64), coinbaseSize: 100,
              newMasternodes: [], deletedMasternodes: [],
              newQuorums: [], deletedQuorums: [],
              chainlockSigCount: 0, chainlockSignatures: [],
            },
          },
        });
      });

      // Verify both diffs exist
      await user.click(screen.getByText("Diffs"));
      const diffList = screen.getByRole("listbox", { name: "Fetched Diffs" });
      expect(within(diffList).getAllByRole("option")).toHaveLength(2);

      // Click Clear Keep Base
      await user.click(screen.getByTestId("clear-keep-base-button"));

      // Should keep only the base diff (baseHeight === 0)
      expect(within(diffList).getAllByRole("option")).toHaveLength(1);
      expect(within(diffList).getByRole("option")).toHaveTextContent("0 → 100");
    });

    it("shows success message after clear keep base", async () => {
      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await user.click(screen.getByTestId("clear-keep-base-button"));
      await screen.findByText("Cleared data, kept base diff");
    });
  });

  // ─── QR Info Routing from Task Results ────────────────────────────────────

  describe("QR Info Routing", () => {
    it("populates QR Info tab when fetched via Get single end QR info", async () => {
      mocks.commands.mnlistFetchQrInfo = vi.fn().mockResolvedValue({
        status: "ok",
        data: { taskId: "task-qr1" },
      });

      const user = userEvent.setup();
      render(<MasternodeListDiffScreen />);
      await flushMicrotasks();

      await user.click(screen.getByTestId("fetch-qrinfo-button"));

      // Simulate QR info result
      act(() => {
        mocks.emitMockEvent("taskResultEvent", {
          taskId: "task-qr1",
          result: {
            type: "mnListFetchedQrInfo",
            qrInfo: {
              tipBlockHash: "tip".padEnd(64, "0"),
              snapshotHMinusC: { skipListMode: 0, activeQuorumMembers: [true], skipList: [] },
              snapshotHMinus2C: { skipListMode: 0, activeQuorumMembers: [], skipList: [] },
              snapshotHMinus3C: { skipListMode: 0, activeQuorumMembers: [], skipList: [] },
              snapshotHMinus4C: null,
              diffHMinus3C: { version: 1, baseBlockHash: "0".repeat(64), blockHash: "1".repeat(64), totalTransactions: 0, merkleHashes: [], merkleFlagsLen: 0, coinbaseTxid: "a".repeat(64), coinbaseSize: 0, newMasternodes: [], deletedMasternodes: [], newQuorums: [], deletedQuorums: [], chainlockSigCount: 0, chainlockSignatures: [] },
              diffHMinus2C: { version: 1, baseBlockHash: "0".repeat(64), blockHash: "1".repeat(64), totalTransactions: 0, merkleHashes: [], merkleFlagsLen: 0, coinbaseTxid: "a".repeat(64), coinbaseSize: 0, newMasternodes: [], deletedMasternodes: [], newQuorums: [], deletedQuorums: [], chainlockSigCount: 0, chainlockSignatures: [] },
              diffHMinusC: { version: 1, baseBlockHash: "0".repeat(64), blockHash: "1".repeat(64), totalTransactions: 0, merkleHashes: [], merkleFlagsLen: 0, coinbaseTxid: "a".repeat(64), coinbaseSize: 0, newMasternodes: [], deletedMasternodes: [], newQuorums: [], deletedQuorums: [], chainlockSigCount: 0, chainlockSignatures: [] },
              diffH: { version: 1, baseBlockHash: "0".repeat(64), blockHash: "1".repeat(64), totalTransactions: 0, merkleHashes: [], merkleFlagsLen: 0, coinbaseTxid: "a".repeat(64), coinbaseSize: 0, newMasternodes: [], deletedMasternodes: [], newQuorums: [], deletedQuorums: [], chainlockSigCount: 0, chainlockSignatures: [] },
              diffTip: { version: 1, baseBlockHash: "0".repeat(64), blockHash: "1".repeat(64), totalTransactions: 0, merkleHashes: [], merkleFlagsLen: 0, coinbaseTxid: "a".repeat(64), coinbaseSize: 0, newMasternodes: [], deletedMasternodes: [], newQuorums: [], deletedQuorums: [], chainlockSigCount: 0, chainlockSignatures: [] },
              diffHMinus4C: null,
              lastCommitments: [],
              quorumSnapshotList: [],
              mnListDiffList: [],
            },
          },
        });
      });

      // Navigate to QR Info tab - data should be populated
      await user.click(screen.getByText("QR Info"));
      expect(screen.getByText(/Tip:/)).toBeInTheDocument();
      expect(screen.getByText("Quorum Snapshots")).toBeInTheDocument();
    });
  });
});
