import { describe, it, expect, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SingleKeyWalletDetail } from "./SingleKeyWalletDetail";
import type { SingleKeyWalletDto, UtxoDto } from "@/bindings";

// ─── Test Fixtures ──────────────────────────────────────────────────

function makeUtxo(overrides: Partial<UtxoDto> = {}): UtxoDto {
  return {
    txid: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
    vout: 0,
    amount: 100000000, // 1 DASH
    ...overrides,
  };
}

function makeWallet(
  overrides: Partial<SingleKeyWalletDto> = {},
): SingleKeyWalletDto {
  return {
    keyHash:
      "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aaaa1111bbbb2222",
    usesPassword: false,
    publicKey: "02abcdef1234567890",
    address: "XsingleKeyAddr1234567890abcdef",
    alias: "My Imported Key",
    confirmedBalance: 500000000, // 5 DASH
    unconfirmedBalance: 0,
    totalBalance: 500000000,
    utxoCount: 2,
    utxos: [
      makeUtxo({ txid: "tx001", vout: 0, amount: 300000000 }),
      makeUtxo({ txid: "tx002", vout: 1, amount: 200000000 }),
    ],
    ...overrides,
  };
}

// Generate many UTXOs for pagination tests
function makeManyUtxos(count: number): UtxoDto[] {
  return Array.from({ length: count }, (_, i) =>
    makeUtxo({
      txid: `tx${String(i).padStart(4, "0")}${"0".repeat(60)}`.slice(0, 64),
      vout: i % 3,
      amount: (count - i) * 1000000, // Descending amounts
    }),
  );
}

// ─── Header Section Tests ──────────────────────────────────────────

describe("SingleKeyWalletDetail", () => {
  describe("header", () => {
    it("renders wallet alias as heading", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(
        "My Imported Key",
      );
    });

    it("renders 'Unnamed Key' when alias is null", () => {
      render(
        <SingleKeyWalletDetail wallet={makeWallet({ alias: null })} />,
      );
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(
        "Unnamed Key",
      );
    });

    it("renders 'Unnamed Key' when alias is empty/whitespace", () => {
      render(
        <SingleKeyWalletDetail wallet={makeWallet({ alias: "   " })} />,
      );
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent(
        "Unnamed Key",
      );
    });

    it("renders the wallet address", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ address: "XtestAddr123" })}
        />,
      );
      expect(screen.getByText("XtestAddr123")).toBeInTheDocument();
    });

    it("renders copy button for the address", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ address: "XcopyableAddr" })}
        />,
      );
      const copyButtons = screen.getAllByLabelText("Copy to clipboard");
      expect(copyButtons.length).toBeGreaterThanOrEqual(1);
    });

    it("renders the balance in DASH", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ totalBalance: 123456789 })}
        />,
      );
      expect(screen.getByText(/Balance:/)).toBeInTheDocument();
      expect(screen.getByText("1.23456789 DASH")).toBeInTheDocument();
    });

    it("renders pending badge when unconfirmed > 0", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ unconfirmedBalance: 10000000 })}
        />,
      );
      expect(screen.getByText(/\+0\.10000000 pending/)).toBeInTheDocument();
    });

    it("does not render pending badge when unconfirmed is 0", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ unconfirmedBalance: 0 })}
        />,
      );
      expect(screen.queryByText(/pending/)).not.toBeInTheDocument();
    });

    it("shows spinner when refreshing", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} refreshing />);
      expect(
        screen.getByLabelText("Refreshing wallet"),
      ).toBeInTheDocument();
    });

    it("does not show spinner when not refreshing", () => {
      render(
        <SingleKeyWalletDetail wallet={makeWallet()} refreshing={false} />,
      );
      expect(
        screen.queryByLabelText("Refreshing wallet"),
      ).not.toBeInTheDocument();
    });
  });

  // ─── Action Bar Tests ──────────────────────────────────────────────

  describe("action bar", () => {
    it("renders Send button when onSend provided", () => {
      const onSend = vi.fn();
      render(<SingleKeyWalletDetail wallet={makeWallet()} onSend={onSend} />);
      expect(
        screen.getByRole("button", { name: /Send/ }),
      ).toBeInTheDocument();
    });

    it("calls onSend when Send is clicked", async () => {
      const user = userEvent.setup();
      const onSend = vi.fn();
      render(<SingleKeyWalletDetail wallet={makeWallet()} onSend={onSend} />);
      await user.click(screen.getByRole("button", { name: /Send/ }));
      expect(onSend).toHaveBeenCalledOnce();
    });

    it("renders Receive button when onReceive provided", () => {
      const onReceive = vi.fn();
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet()}
          onReceive={onReceive}
        />,
      );
      expect(
        screen.getByRole("button", { name: /^Receive$/ }),
      ).toBeInTheDocument();
    });

    it("calls onReceive when Receive is clicked", async () => {
      const user = userEvent.setup();
      const onReceive = vi.fn();
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet()}
          onReceive={onReceive}
        />,
      );
      await user.click(screen.getByRole("button", { name: /^Receive$/ }));
      expect(onReceive).toHaveBeenCalledOnce();
    });

    it("renders Refresh button when onRefresh provided", () => {
      const onRefresh = vi.fn();
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet()}
          onRefresh={onRefresh}
        />,
      );
      expect(
        screen.getByRole("button", { name: /Refresh/ }),
      ).toBeInTheDocument();
    });

    it("calls onRefresh when Refresh is clicked", async () => {
      const user = userEvent.setup();
      const onRefresh = vi.fn();
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet()}
          onRefresh={onRefresh}
        />,
      );
      await user.click(screen.getByRole("button", { name: /Refresh/ }));
      expect(onRefresh).toHaveBeenCalledOnce();
    });

    it("disables Refresh button when refreshing", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet()}
          onRefresh={() => {}}
          refreshing
        />,
      );
      expect(
        screen.getByRole("button", { name: /Refresh/ }),
      ).toBeDisabled();
    });

    it("does not show buttons when callbacks are not provided", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(
        screen.queryByRole("button", { name: /^Send$/ }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /^Receive$/ }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /^Refresh$/ }),
      ).not.toBeInTheDocument();
    });
  });

  // ─── UTXO Section Tests ──────────────────────────────────────────

  describe("UTXO section", () => {
    it("renders UTXO count in heading", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(screen.getByText("UTXOs (2)")).toBeInTheDocument();
    });

    it("renders UTXO cards", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(screen.getByText(/tx001:0/)).toBeInTheDocument();
      expect(screen.getByText(/tx002:1/)).toBeInTheDocument();
    });

    it("renders UTXO amounts in DASH", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(screen.getByText("3.00000000 DASH")).toBeInTheDocument();
      expect(screen.getByText("2.00000000 DASH")).toBeInTheDocument();
    });

    it("renders copy button for each UTXO", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      // Address copy + 2 UTXO copies = at least 3
      const copyButtons = screen.getAllByLabelText("Copy to clipboard");
      expect(copyButtons.length).toBeGreaterThanOrEqual(3);
    });

    it("shows empty state when no UTXOs", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ utxos: [], utxoCount: 0 })}
        />,
      );
      expect(screen.getByText("No UTXOs available")).toBeInTheDocument();
      expect(
        screen.getByText(/Click 'Refresh' to load UTXOs from Core/),
      ).toBeInTheDocument();
    });

    it("shows zero UTXO count in heading for empty wallet", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ utxos: [], utxoCount: 0 })}
        />,
      );
      expect(screen.getByText("UTXOs (0)")).toBeInTheDocument();
    });

    it("sorts UTXOs by amount descending", () => {
      const wallet = makeWallet({
        utxos: [
          makeUtxo({ txid: "small", vout: 0, amount: 1000 }),
          makeUtxo({ txid: "large", vout: 0, amount: 999000000 }),
          makeUtxo({ txid: "medium", vout: 0, amount: 50000000 }),
        ],
        utxoCount: 3,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      // Get UTXO card amounts via data-testid containers
      const utxoCards = screen.getAllByTestId("utxo-card");
      const amounts = utxoCards.map(
        (card) => within(card).getByText(/DASH$/).textContent,
      );
      // First should be largest
      expect(amounts[0]).toBe("9.99000000 DASH");
      expect(amounts[1]).toBe("0.50000000 DASH");
      expect(amounts[2]).toBe("0.00001000 DASH");
    });
  });

  // ─── Pagination Tests ────────────────────────────────────────────

  describe("pagination", () => {
    it("does not show pagination when UTXOs fit in one page", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(screen.queryByText(/Page \d+ of \d+/)).not.toBeInTheDocument();
      expect(
        screen.queryByRole("navigation", { name: /pagination/i }),
      ).not.toBeInTheDocument();
    });

    it("shows pagination when UTXOs exceed one page", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);
      expect(screen.getByText(/Page 1 of 2/)).toBeInTheDocument();
      expect(screen.getByText(/1-50 of 75/)).toBeInTheDocument();
    });

    it("navigates to next page", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      await user.click(screen.getAllByRole("button", { name: /Next page/ })[0]);
      expect(screen.getByText(/Page 2 of 2/)).toBeInTheDocument();
      expect(screen.getByText(/51-75 of 75/)).toBeInTheDocument();
    });

    it("navigates to previous page", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      // Go to page 2 then back to page 1
      await user.click(screen.getAllByRole("button", { name: /Next page/ })[0]);
      await user.click(screen.getAllByRole("button", { name: /Previous page/ })[0]);
      expect(screen.getByText(/Page 1 of 2/)).toBeInTheDocument();
    });

    it("navigates to first page", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        utxos: makeManyUtxos(150),
        utxoCount: 150,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      // Go to last page then back to first
      await user.click(screen.getAllByRole("button", { name: /Last page/ })[0]);
      expect(screen.getAllByText(/Page 3 of 3/).length).toBeGreaterThanOrEqual(1);
      await user.click(screen.getAllByRole("button", { name: /First page/ })[0]);
      expect(screen.getAllByText(/Page 1 of 3/).length).toBeGreaterThanOrEqual(1);
    });

    it("navigates to last page", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        utxos: makeManyUtxos(150),
        utxoCount: 150,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      await user.click(screen.getAllByRole("button", { name: /Last page/ })[0]);
      expect(screen.getAllByText(/Page 3 of 3/).length).toBeGreaterThanOrEqual(1);
      expect(screen.getAllByText(/101-150 of 150/).length).toBeGreaterThanOrEqual(1);
    });

    it("disables First and Prev on first page", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      const firstBtns = screen.getAllByRole("button", { name: /First page/ });
      const prevBtns = screen.getAllByRole("button", { name: /Previous page/ });
      expect(firstBtns[0]).toBeDisabled();
      expect(prevBtns[0]).toBeDisabled();
    });

    it("disables Next and Last on last page", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      await user.click(screen.getAllByRole("button", { name: /Last page/ })[0]);
      const nextBtns = screen.getAllByRole("button", { name: /Next page/ });
      const lastBtns = screen.getAllByRole("button", { name: /Last page/ });
      expect(nextBtns[0]).toBeDisabled();
      expect(lastBtns[0]).toBeDisabled();
    });

    it("shows exactly 50 UTXOs per page", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      const utxoCards = screen.getAllByTestId("utxo-card");
      expect(utxoCards.length).toBe(50);
    });

    it("shows remaining UTXOs on last page", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      await user.click(screen.getAllByRole("button", { name: /Next page/ })[0]);
      const utxoCards = screen.getAllByTestId("utxo-card");
      expect(utxoCards.length).toBe(25);
    });

    it("shows bottom pagination for 3+ pages", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(150),
        utxoCount: 150,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      // Should have 2 pagination navigations (top + bottom)
      const paginationNavs = screen.getAllByRole("navigation", {
        name: /pagination/i,
      });
      expect(paginationNavs.length).toBe(2);
    });

    it("does not show bottom pagination for exactly 2 pages", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);

      // Should have 1 pagination navigation (top only)
      const paginationNavs = screen.getAllByRole("navigation", {
        name: /pagination/i,
      });
      expect(paginationNavs.length).toBe(1);
    });
  });

  // ─── Balance Display Tests ──────────────────────────────────────

  describe("balance display", () => {
    it("formats zero balance correctly", () => {
      render(
        <SingleKeyWalletDetail wallet={makeWallet({ totalBalance: 0 })} />,
      );
      expect(screen.getByText("0.00000000 DASH")).toBeInTheDocument();
    });

    it("formats large balance correctly", () => {
      render(
        <SingleKeyWalletDetail
          wallet={makeWallet({ totalBalance: 2100000000000000 })}
        />,
      );
      expect(
        screen.getByText("21000000.00000000 DASH"),
      ).toBeInTheDocument();
    });

    it("formats small balance correctly", () => {
      render(
        <SingleKeyWalletDetail wallet={makeWallet({ totalBalance: 1 })} />,
      );
      expect(screen.getByText("0.00000001 DASH")).toBeInTheDocument();
    });
  });

  // ─── Accessibility Tests ────────────────────────────────────────

  describe("accessibility", () => {
    it("uses heading level 2 for wallet name", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(
        screen.getByRole("heading", { level: 2 }),
      ).toBeInTheDocument();
    });

    it("uses heading level 3 for UTXO section", () => {
      render(<SingleKeyWalletDetail wallet={makeWallet()} />);
      expect(
        screen.getByRole("heading", { level: 3 }),
      ).toHaveTextContent(/UTXOs/);
    });

    it("pagination has navigation role", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);
      expect(
        screen.getByRole("navigation", { name: /pagination/i }),
      ).toBeInTheDocument();
    });

    it("pagination buttons have aria-labels", () => {
      const wallet = makeWallet({
        utxos: makeManyUtxos(75),
        utxoCount: 75,
      });
      render(<SingleKeyWalletDetail wallet={wallet} />);
      expect(screen.getAllByLabelText("First page")[0]).toBeInTheDocument();
      expect(screen.getAllByLabelText("Previous page")[0]).toBeInTheDocument();
      expect(screen.getAllByLabelText("Next page")[0]).toBeInTheDocument();
      expect(screen.getAllByLabelText("Last page")[0]).toBeInTheDocument();
    });
  });

  // ─── Custom className Tests ─────────────────────────────────────

  describe("className", () => {
    it("applies custom className", () => {
      const { container } = render(
        <SingleKeyWalletDetail
          wallet={makeWallet()}
          className="my-custom-class"
        />,
      );
      expect(container.firstChild).toHaveClass("my-custom-class");
    });
  });
});
