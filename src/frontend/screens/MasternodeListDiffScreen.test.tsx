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

  it("enables QR Info tab and disables Quorum Viewer tab", () => {
    render(<MasternodeListDiffScreen />);
    const qrInfoTab = screen.getByText("QR Info").closest("button");
    const quorumTab = screen.getByText("Quorum Viewer").closest("button");
    expect(qrInfoTab).not.toBeDisabled();
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
});
