import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  ReviewStep,
  estimateRegistrationCost,
  isPermanentlyNonTradeable,
  buildConfigJson,
  formatCreditsAsDash,
  getActionTakerLabel,
  getDistributionFunctionLabel,
  getIntervalTypeLabel,
} from "./ReviewStep";
import type { ReviewStepProps } from "./ReviewStep";
import { createDefaultBasicInfo } from "./TokenCreatorWizard";
import type { BasicInfoState } from "./TokenCreatorWizard";
import { createDefaultDistributionState } from "./DistributionStep";
import { createDefaultControlRulesState } from "./ControlRulesStep";
import { createDefaultGroupsState } from "./GroupsStep";
import { createDefaultHistoryState } from "./HistoryStep";
import { createDefaultKeywordsState } from "./KeywordsStep";

// ─── Mocks ──────────────────────────────────────────────────────────

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands } from "@/bindings";

const mockIdentities = [
  {
    id: "id-abc123def456",
    alias: "TestIdentity",
    balance: 1000000000,
    keys: [
      {
        keyId: 1,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        isDisabled: false,
      },
      {
        keyId: 2,
        purpose: "AUTHENTICATION",
        securityLevel: "CRITICAL",
        keyType: "ECDSA_SECP256K1",
        isDisabled: false,
      },
    ],
    associatedWalletHashes: ["seed-hash-1"],
    dpnsNames: [],
    identityType: "User" as const,
    walletIndex: 0,
    topUps: [],
    status: "NotInPlatform" as const,
  },
];

const mockLoadIdentities = vi.fn();
const mockLoadWallets = vi.fn();
const mockLoadMyTokenBalances = vi.fn().mockResolvedValue(null);

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      identities: mockIdentities,
      loadIdentities: mockLoadIdentities,
    };
    return sel ? sel(s) : s;
  },
}));

vi.mock("@/stores/walletStore", () => ({
  useWalletStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      hdWallets: [
        {
          seedHash: "seed-hash-1",
          alias: "TestWallet",
          usesPassword: false,
          passwordHint: null,
        },
      ],
      singleKeyWallets: [],
      loadWallets: mockLoadWallets,
    };
    return sel ? sel(s) : s;
  },
}));

vi.mock("@/stores/tokenStore", () => ({
  useTokenStore: () => ({
    loadMyTokenBalances: mockLoadMyTokenBalances,
  }),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

vi.mock("@/components/shared/WalletUnlockDialog", () => ({
  WalletUnlockDialog: () => <div data-testid="wallet-unlock-dialog-mock" />,
}));

// ─── Helpers ────────────────────────────────────────────────────────

function validBasicInfo(
  overrides: Partial<BasicInfoState> = {},
): BasicInfoState {
  return {
    ...createDefaultBasicInfo(),
    names: [
      { singular: "TestToken", plural: "TestTokens", languageCode: "en" },
    ],
    baseSupply: "1000000",
    ...overrides,
  };
}

function defaultProps(
  overrides: Partial<ReviewStepProps> = {},
): ReviewStepProps {
  return {
    basicInfo: validBasicInfo(),
    distribution: createDefaultDistributionState(),
    controlRules: createDefaultControlRulesState(),
    groups: createDefaultGroupsState(),
    history: createDefaultHistoryState(),
    keywords: createDefaultKeywordsState(),
    onReset: vi.fn(),
    ...overrides,
  };
}

// ─── formatCreditsAsDash ────────────────────────────────────────────

describe("formatCreditsAsDash", () => {
  it("formats 0 credits", () => {
    expect(formatCreditsAsDash(0)).toBe("~0.000000 DASH");
  });

  it("formats 100 billion credits as 1 DASH", () => {
    expect(formatCreditsAsDash(100_000_000_000)).toBe("~1.000000 DASH");
  });

  it("formats small amounts", () => {
    expect(formatCreditsAsDash(50_000_000)).toBe("~0.000500 DASH");
  });
});

// ─── getActionTakerLabel ────────────────────────────────────────────

describe("getActionTakerLabel", () => {
  it("returns 'No One' for noOne", () => {
    expect(getActionTakerLabel("noOne")).toBe("No One");
  });

  it("returns 'Contract Owner' for contractOwner", () => {
    expect(getActionTakerLabel("contractOwner")).toBe("Contract Owner");
  });

  it("returns 'Specific Identity' for identity", () => {
    expect(getActionTakerLabel("identity")).toBe("Specific Identity");
  });

  it("returns 'Main Group' for mainGroup", () => {
    expect(getActionTakerLabel("mainGroup")).toBe("Main Group");
  });

  it("returns 'Specific Group' for group", () => {
    expect(getActionTakerLabel("group")).toBe("Specific Group");
  });
});

// ─── getDistributionFunctionLabel ───────────────────────────────────

describe("getDistributionFunctionLabel", () => {
  it("returns label for all 8 function types", () => {
    expect(getDistributionFunctionLabel("fixedAmount")).toBe("Fixed Amount");
    expect(getDistributionFunctionLabel("stepDecreasing")).toBe(
      "Step Decreasing",
    );
    expect(getDistributionFunctionLabel("stepwise")).toBe("Stepwise");
    expect(getDistributionFunctionLabel("linear")).toBe("Linear");
    expect(getDistributionFunctionLabel("polynomial")).toBe("Polynomial");
    expect(getDistributionFunctionLabel("exponential")).toBe("Exponential");
    expect(getDistributionFunctionLabel("logarithmic")).toBe("Logarithmic");
    expect(getDistributionFunctionLabel("invertedLogarithmic")).toBe(
      "Inverted Logarithmic",
    );
  });

  it("returns raw string for unknown types", () => {
    expect(getDistributionFunctionLabel("unknown")).toBe("unknown");
  });
});

// ─── getIntervalTypeLabel ───────────────────────────────────────────

describe("getIntervalTypeLabel", () => {
  it("returns labels for all interval types", () => {
    expect(getIntervalTypeLabel("timeBased")).toBe("Time-based");
    expect(getIntervalTypeLabel("blockBased")).toBe("Block-based");
    expect(getIntervalTypeLabel("epochBased")).toBe("Epoch-based");
  });

  it("returns raw string for unknown types", () => {
    expect(getIntervalTypeLabel("custom")).toBe("custom");
  });
});

// ─── estimateRegistrationCost ───────────────────────────────────────

describe("estimateRegistrationCost", () => {
  it("returns base cost for minimal config", () => {
    const cost = estimateRegistrationCost(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultKeywordsState(),
    );
    // Base: 50M + Token: 100M + 1 searchable name: 10M + buffer: 200M = 360M
    expect(cost).toBe(360_000_000);
  });

  it("adds perpetual distribution fee", () => {
    const dist = createDefaultDistributionState();
    dist.perpetual.enabled = true;
    const cost = estimateRegistrationCost(
      validBasicInfo(),
      dist,
      createDefaultKeywordsState(),
    );
    expect(cost).toBe(360_000_000 + 50_000_000);
  });

  it("adds pre-programmed distribution fee", () => {
    const dist = createDefaultDistributionState();
    dist.preProgrammed.enabled = true;
    const cost = estimateRegistrationCost(
      validBasicInfo(),
      dist,
      createDefaultKeywordsState(),
    );
    expect(cost).toBe(360_000_000 + 50_000_000);
  });

  it("adds both distribution fees", () => {
    const dist = createDefaultDistributionState();
    dist.perpetual.enabled = true;
    dist.preProgrammed.enabled = true;
    const cost = estimateRegistrationCost(
      validBasicInfo(),
      dist,
      createDefaultKeywordsState(),
    );
    expect(cost).toBe(360_000_000 + 100_000_000);
  });

  it("adds contract keyword fees", () => {
    const basic = validBasicInfo({ contractKeywords: "gaming, reward" });
    const cost = estimateRegistrationCost(
      basic,
      createDefaultDistributionState(),
      createDefaultKeywordsState(),
    );
    // 360M base + 2 * 10M contract keywords = 380M
    expect(cost).toBe(380_000_000);
  });

  it("adds token keyword fees", () => {
    const kw = createDefaultKeywordsState();
    kw.keywords = ["tag1", "tag2", "tag3"];
    const cost = estimateRegistrationCost(
      validBasicInfo(),
      createDefaultDistributionState(),
      kw,
    );
    // 360M base + 3 * 10M token keywords = 390M
    expect(cost).toBe(390_000_000);
  });

  it("adds additional language name fees", () => {
    const basic = validBasicInfo({
      names: [
        { singular: "TestToken", plural: "TestTokens", languageCode: "en" },
        { singular: "Jeton", plural: "Jetons", languageCode: "fr" },
      ],
    });
    const cost = estimateRegistrationCost(
      basic,
      createDefaultDistributionState(),
      createDefaultKeywordsState(),
    );
    // 360M base + 1 additional searchable name = 370M
    expect(cost).toBe(370_000_000);
  });

  it("ignores contract keywords shorter than 3 chars", () => {
    const basic = validBasicInfo({ contractKeywords: "ab, gaming" });
    const cost = estimateRegistrationCost(
      basic,
      createDefaultDistributionState(),
      createDefaultKeywordsState(),
    );
    // 360M + 1 * 10M = 370M (ab is ignored)
    expect(cost).toBe(370_000_000);
  });
});

// ─── isPermanentlyNonTradeable ──────────────────────────────────────

describe("isPermanentlyNonTradeable", () => {
  it("returns false for default control rules (noOne but changingAllowed true)", () => {
    const rules = createDefaultControlRulesState();
    expect(isPermanentlyNonTradeable(rules)).toBe(false);
  });

  it("returns true when marketplace is noOne and changingAuthorized is false", () => {
    const rules = createDefaultControlRulesState();
    rules.marketplace.authorizedActionTaker = "noOne";
    rules.marketplace.changingAuthorizedToNoOneAllowed = false;
    expect(isPermanentlyNonTradeable(rules)).toBe(true);
  });

  it("returns false when marketplace is contractOwner", () => {
    const rules = createDefaultControlRulesState();
    rules.marketplace.authorizedActionTaker = "contractOwner";
    rules.marketplace.changingAuthorizedToNoOneAllowed = false;
    expect(isPermanentlyNonTradeable(rules)).toBe(false);
  });
});

// ─── buildConfigJson ────────────────────────────────────────────────

describe("buildConfigJson", () => {
  it("serializes basic info with backend field names", () => {
    const config = buildConfigJson(
      validBasicInfo({ description: "A test token" }),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    // tokenNames as array of tuples
    expect(config.tokenNames).toEqual([["TestToken", "TestTokens", "en"]]);
    expect(config.tokenDescription).toBe("A test token");
    expect(config.baseSupply).toBe("1000000");
    expect(config.maxSupply).toBeNull();
    expect(config.decimals).toBe(8);
    expect(config.shouldCapitalize).toBe(false);
    expect(config.startPaused).toBe(false);
    expect(config.allowTransfersToFrozenIdentities).toBe(false);
  });

  it("serializes contract keywords as array", () => {
    const config = buildConfigJson(
      validBasicInfo({ contractKeywords: "gaming, reward" }),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    expect(config.contractKeywords).toEqual(["gaming", "reward"]);
  });

  it("serializes null distributions in distributionRules when disabled", () => {
    const config = buildConfigJson(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    const distRules = config.distributionRules as Record<string, unknown>;
    expect(distRules.$format_version).toBe("0");
    expect(distRules.perpetualDistribution).toBeNull();
    expect(distRules.preProgrammedDistribution).toBeNull();
  });

  it("serializes perpetual distribution when enabled", () => {
    const dist = createDefaultDistributionState();
    dist.perpetual.enabled = true;
    dist.perpetual.distributionFunction = "linear";
    dist.perpetual.intervalType = "blockBased";
    dist.perpetual.intervalAmount = "10";
    const config = buildConfigJson(
      validBasicInfo(),
      dist,
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    const distRules = config.distributionRules as Record<string, unknown>;
    expect(distRules.perpetualDistribution).not.toBeNull();
    const perp = distRules.perpetualDistribution as Record<string, unknown>;
    expect(perp.$format_version).toBe("0");
    const distType = perp.distributionType as Record<string, unknown>;
    expect(distType).toHaveProperty("BlockBasedDistribution");
    const block = distType.BlockBasedDistribution as Record<string, unknown>;
    expect(block.interval).toBe(10);
    expect(block.function).toHaveProperty("Linear");
  });

  it("serializes control rules as top-level V0 structs", () => {
    const config = buildConfigJson(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    // Control rules are now top-level keys with V0-wrapped values
    const mintRules = config.manualMintingRules as Record<string, unknown>;
    const v0 = mintRules.V0 as Record<string, unknown>;
    expect(v0.authorized_to_make_change).toBe("NoOne");
    expect(v0.admin_action_takers).toBe("NoOne");

    const marketRules = config.marketplaceRules as Record<string, unknown>;
    const mv0 = marketRules.V0 as Record<string, unknown>;
    expect(mv0.authorized_to_make_change).toBe("NoOne");
  });

  it("serializes mainControlGroupChangeAuthorized as AuthorizedActionTakers", () => {
    const config = buildConfigJson(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    expect(config.mainControlGroupChangeAuthorized).toBe("NoOne");
  });

  it("serializes groups as position-keyed object", () => {
    const groups = createDefaultGroupsState();
    groups.groups = [
      {
        requiredPower: "100",
        members: [{ identityId: "abc123", power: "50" }],
        expanded: true,
      },
    ];
    const config = buildConfigJson(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      groups,
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    const configGroups = config.groups as Record<string, Record<string, unknown>>;
    expect(configGroups["0"]).toBeDefined();
    expect(configGroups["0"].$format_version).toBe("0");
    expect(configGroups["0"].required_power).toBe(100);
    const members = configGroups["0"].members as Record<string, number>;
    expect(members["abc123"]).toBe(50);
  });

  it("serializes keepsHistory with Platform SDK format", () => {
    const history = createDefaultHistoryState();
    history.keepTransferHistory = false;
    const config = buildConfigJson(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      history,
      createDefaultKeywordsState(),
    );
    const keepHist = config.keepsHistory as Record<string, unknown>;
    expect(keepHist.$format_version).toBe("0");
    expect(keepHist.keepsTransferHistory).toBe(false);
    expect(keepHist.keepsMintingHistory).toBe(true);
  });

  it("includes marketplaceTradeMode", () => {
    const config = buildConfigJson(
      validBasicInfo(),
      createDefaultDistributionState(),
      createDefaultControlRulesState(),
      createDefaultGroupsState(),
      createDefaultHistoryState(),
      createDefaultKeywordsState(),
    );
    expect(config.marketplaceTradeMode).toBe(0);
  });
});

// ─── ReviewStep component tests ─────────────────────────────────────

describe("ReviewStep", () => {
  let onReset: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    onReset = vi.fn();
    vi.mocked(commands.tokenRegisterContract).mockResolvedValue({
      status: "ok",
      data: { taskId: "task-1" },
    });
    vi.mocked(commands.walletNotifyUnlocked).mockResolvedValue({
      status: "ok",
      data: null,
    });
  });

  it("renders the review step container", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByTestId("review-step")).toBeInTheDocument();
  });

  it("loads identities and wallets on mount", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(mockLoadIdentities).toHaveBeenCalledTimes(1);
    expect(mockLoadWallets).toHaveBeenCalledTimes(1);
  });

  it("renders identity selector", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByTestId("review-identity-select")).toBeInTheDocument();
  });

  it("renders key selector", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByTestId("review-key-select")).toBeInTheDocument();
  });

  it("shows wallet unlocked status when wallet is not password-protected", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByText("Wallet unlocked")).toBeInTheDocument();
  });

  it("renders fee estimation section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByTestId("fee-estimation")).toBeInTheDocument();
    expect(
      screen.getByText("Estimated Registration Cost"),
    ).toBeInTheDocument();
  });

  it("renders create button", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByTestId("review-create-button")).toBeInTheDocument();
    expect(screen.getByTestId("review-create-button")).not.toBeDisabled();
  });

  // ── Summary sections ──────────────────────────────────────────────

  it("renders Basic Info summary section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(
      screen.getByTestId("summary-section-basicInfo"),
    ).toBeInTheDocument();
  });

  it("renders token name in summary", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByText("TestToken")).toBeInTheDocument();
  });

  it("renders base supply in summary", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByText("1000000")).toBeInTheDocument();
  });

  it("shows 'Unlimited' for empty max supply", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.getByText("Unlimited")).toBeInTheDocument();
  });

  it("renders Distribution summary section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    const section = screen.getByTestId("summary-section-distribution");
    expect(section).toBeInTheDocument();
  });

  it("renders Control Rules summary section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    const section = screen.getByTestId("summary-section-controlRules");
    expect(section).toBeInTheDocument();
  });

  it("renders Groups summary section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    const section = screen.getByTestId("summary-section-groups");
    expect(section).toBeInTheDocument();
  });

  it("renders History summary section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    const section = screen.getByTestId("summary-section-history");
    expect(section).toBeInTheDocument();
  });

  it("renders Keywords summary section", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    const section = screen.getByTestId("summary-section-keywords");
    expect(section).toBeInTheDocument();
  });

  // ── Section toggles ───────────────────────────────────────────────

  it("toggles distribution section on click", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    const toggle = screen.getByTestId("summary-toggle-distribution");
    await user.click(toggle);
    // After expanding, should show "No distribution configured" since defaults are disabled
    expect(
      screen.getByText("No distribution configured"),
    ).toBeInTheDocument();
  });

  it("toggles groups section showing 'No groups configured'", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("summary-toggle-groups"));
    expect(screen.getByText("No groups configured")).toBeInTheDocument();
  });

  it("toggles keywords section showing 'No keywords added'", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("summary-toggle-keywords"));
    expect(screen.getByText("No keywords added")).toBeInTheDocument();
  });

  it("shows history values when expanded", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("summary-toggle-history"));
    // Default history keeps all
    expect(screen.getAllByText("Keep")).toHaveLength(6);
  });

  it("shows 'Skip' for disabled history flags", async () => {
    const user = userEvent.setup();
    const history = createDefaultHistoryState();
    history.keepTransferHistory = false;
    history.keepBurningHistory = false;
    render(
      <ReviewStep {...defaultProps({ onReset, history })} />,
    );
    await user.click(screen.getByTestId("summary-toggle-history"));
    expect(screen.getAllByText("Skip")).toHaveLength(2);
    expect(screen.getAllByText("Keep")).toHaveLength(4);
  });

  it("shows control rules summary with action taker labels", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("summary-toggle-controlRules"));
    // Default is "No One" for all rules
    const section = screen.getByTestId("summary-section-controlRules");
    const noOnes = within(section).getAllByText("No One");
    expect(noOnes.length).toBeGreaterThanOrEqual(9);
  });

  it("shows keywords as badges when present", async () => {
    const user = userEvent.setup();
    const keywords = createDefaultKeywordsState();
    keywords.keywords = ["gaming", "reward"];
    render(
      <ReviewStep {...defaultProps({ onReset, keywords })} />,
    );
    await user.click(screen.getByTestId("summary-toggle-keywords"));
    expect(screen.getByText("gaming")).toBeInTheDocument();
    expect(screen.getByText("reward")).toBeInTheDocument();
  });

  it("shows group info when groups are configured", async () => {
    const user = userEvent.setup();
    const groups = createDefaultGroupsState();
    groups.groups = [
      {
        requiredPower: "100",
        members: [{ identityId: "abc", power: "50" }],
        expanded: false,
      },
    ];
    render(
      <ReviewStep {...defaultProps({ onReset, groups })} />,
    );
    await user.click(screen.getByTestId("summary-toggle-groups"));
    expect(
      screen.getByText("Required Power: 100, Members: 1"),
    ).toBeInTheDocument();
  });

  it("shows distribution info when perpetual is enabled", async () => {
    const user = userEvent.setup();
    const dist = createDefaultDistributionState();
    dist.perpetual.enabled = true;
    dist.perpetual.distributionFunction = "linear";
    render(
      <ReviewStep
        {...defaultProps({ onReset, distribution: dist })}
      />,
    );
    await user.click(screen.getByTestId("summary-toggle-distribution"));
    expect(screen.getByText("Enabled")).toBeInTheDocument();
    expect(screen.getByText("Linear")).toBeInTheDocument();
  });

  // ── Danger mode ───────────────────────────────────────────────────

  it("does not show danger warning by default", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(screen.queryByTestId("danger-warning")).not.toBeInTheDocument();
  });

  it("shows danger warning when permanently non-tradeable", () => {
    const rules = createDefaultControlRulesState();
    rules.marketplace.authorizedActionTaker = "noOne";
    rules.marketplace.changingAuthorizedToNoOneAllowed = false;
    render(
      <ReviewStep
        {...defaultProps({ onReset, controlRules: rules })}
      />,
    );
    expect(screen.getByTestId("danger-warning")).toBeInTheDocument();
    expect(
      screen.getByText("Permanently Non-Tradeable"),
    ).toBeInTheDocument();
  });

  // ── Confirmation dialog ───────────────────────────────────────────

  it("opens confirmation dialog when create button is clicked", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    expect(screen.getByTestId("review-confirm-dialog")).toBeInTheDocument();
    expect(
      screen.getByText("Confirm Token Contract Registration"),
    ).toBeInTheDocument();
  });

  it("shows token name in confirmation dialog", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    const dialog = screen.getByTestId("review-confirm-dialog");
    expect(within(dialog).getByText(/TestToken/)).toBeInTheDocument();
  });

  it("closes confirmation dialog on cancel", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    expect(screen.getByTestId("review-confirm-dialog")).toBeInTheDocument();
    await user.click(screen.getByTestId("review-confirm-cancel"));
    expect(
      screen.queryByTestId("review-confirm-dialog"),
    ).not.toBeInTheDocument();
  });

  it("calls tokenRegisterContract on confirm", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    await user.click(screen.getByTestId("review-confirm-submit"));

    expect(vi.mocked(commands.tokenRegisterContract)).toHaveBeenCalledTimes(1);
    const args = vi.mocked(commands.tokenRegisterContract).mock.calls[0][0];
    expect(args.identityId).toBe("id-abc123def456");
    expect(args.keyId).toBe(1); // HIGH key auto-selected
  });

  it("shows broadcasting state after confirm", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    await user.click(screen.getByTestId("review-confirm-submit"));

    expect(screen.getByTestId("review-broadcasting")).toBeInTheDocument();
    expect(
      screen.getByText("Registering Token Contract..."),
    ).toBeInTheDocument();
  });

  it("shows error state when IPC returns error", async () => {
    vi.mocked(commands.tokenRegisterContract).mockResolvedValueOnce({
      status: "error",
      error: "Insufficient balance",
    });
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    await user.click(screen.getByTestId("review-confirm-submit"));

    expect(screen.getByTestId("review-error")).toBeInTheDocument();
    expect(screen.getByText("Registration Failed")).toBeInTheDocument();
    expect(screen.getByText("Insufficient balance")).toBeInTheDocument();
  });

  it("shows try again button on error", async () => {
    vi.mocked(commands.tokenRegisterContract).mockResolvedValueOnce({
      status: "error",
      error: "Network error",
    });
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    await user.click(screen.getByTestId("review-confirm-submit"));

    expect(screen.getByTestId("review-try-again")).toBeInTheDocument();
  });

  it("returns to idle state when Try Again is clicked", async () => {
    vi.mocked(commands.tokenRegisterContract).mockResolvedValueOnce({
      status: "error",
      error: "Network error",
    });
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    await user.click(screen.getByTestId("review-confirm-submit"));

    await user.click(screen.getByTestId("review-try-again"));
    expect(screen.getByTestId("review-step")).toBeInTheDocument();
  });

  it("shows error when IPC throws", async () => {
    vi.mocked(commands.tokenRegisterContract).mockRejectedValueOnce(
      new Error("Connection lost"),
    );
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-create-button"));
    await user.click(screen.getByTestId("review-confirm-submit"));

    expect(screen.getByTestId("review-error")).toBeInTheDocument();
    expect(screen.getByText("Connection lost")).toBeInTheDocument();
  });

  // ── Additional languages ──────────────────────────────────────────

  it("shows additional languages in summary", () => {
    const basic = validBasicInfo({
      names: [
        { singular: "TestToken", plural: "TestTokens", languageCode: "en" },
        { singular: "Jeton", plural: "Jetons", languageCode: "fr" },
      ],
    });
    render(
      <ReviewStep {...defaultProps({ onReset, basicInfo: basic })} />,
    );
    expect(screen.getByText(/French: Jeton/)).toBeInTheDocument();
  });

  // ── Description ───────────────────────────────────────────────────

  it("shows description in summary when present", () => {
    const basic = validBasicInfo({ description: "My special token" });
    render(
      <ReviewStep {...defaultProps({ onReset, basicInfo: basic })} />,
    );
    expect(screen.getByText("My special token")).toBeInTheDocument();
  });

  // ── Max supply ────────────────────────────────────────────────────

  it("shows max supply when set", () => {
    const basic = validBasicInfo({ maxSupply: "5000000" });
    render(
      <ReviewStep {...defaultProps({ onReset, basicInfo: basic })} />,
    );
    expect(screen.getByText("5000000")).toBeInTheDocument();
  });

  // ── View Contract JSON ────────────────────────────────────────────

  it("renders View Contract JSON button", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(
      screen.getByTestId("review-view-json-button"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("View Contract JSON"),
    ).toBeInTheDocument();
  });

  it("opens JSON preview dialog with pretty-printed JSON on click", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-view-json-button"));

    expect(screen.getByTestId("json-preview-dialog")).toBeInTheDocument();
    expect(screen.getByText("Data Contract JSON")).toBeInTheDocument();
    // The JSON should contain the token name inside the pre block
    const dialog = screen.getByTestId("json-preview-dialog");
    const pre = dialog.querySelector("pre");
    expect(pre?.textContent).toContain("TestToken");
    expect(pre?.textContent).toContain("1000000");
  });

  it("shows copy button in JSON preview dialog", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-view-json-button"));

    expect(screen.getByTestId("json-preview-copy")).toBeInTheDocument();
    expect(screen.getByText("Copy JSON")).toBeInTheDocument();
  });

  it("shows Copied state after clicking copy button", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-view-json-button"));

    // Initially shows "Copy JSON"
    expect(screen.getByText("Copy JSON")).toBeInTheDocument();
    await user.click(screen.getByTestId("json-preview-copy"));

    // After successful copy, button text changes to "Copied"
    expect(screen.getByText("Copied")).toBeInTheDocument();
  });

  it("closes JSON preview dialog via Close button", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-view-json-button"));
    expect(screen.getByTestId("json-preview-dialog")).toBeInTheDocument();

    await user.click(screen.getByTestId("json-preview-close"));
    expect(
      screen.queryByTestId("json-preview-dialog"),
    ).not.toBeInTheDocument();
  });

  // ── Calculate Fee ─────────────────────────────────────────────────

  it("renders Calculate Fee button", () => {
    render(<ReviewStep {...defaultProps({ onReset })} />);
    expect(
      screen.getByTestId("review-calculate-fee-button"),
    ).toBeInTheDocument();
    expect(screen.getByText("Calculate Fee")).toBeInTheDocument();
  });

  it("opens fee breakdown dialog on click", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-calculate-fee-button"));

    expect(screen.getByTestId("fee-dialog")).toBeInTheDocument();
    expect(
      screen.getByText("Fee Estimation Breakdown"),
    ).toBeInTheDocument();
  });

  it("shows fee breakdown line items", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-calculate-fee-button"));

    const breakdown = screen.getByTestId("fee-breakdown");
    expect(
      within(breakdown).getByText("Base contract registration"),
    ).toBeInTheDocument();
    expect(
      within(breakdown).getByText("Token registration"),
    ).toBeInTheDocument();
    expect(
      within(breakdown).getByText("Estimation buffer"),
    ).toBeInTheDocument();
    expect(
      within(breakdown).getByText(/Searchable token names/),
    ).toBeInTheDocument();
  });

  it("shows correct total in fee breakdown", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-calculate-fee-button"));

    const total = screen.getByTestId("fee-breakdown-total");
    // 50M base + 100M token + 10M name + 200M buffer = 360M credits = ~0.003600 DASH
    expect(total.textContent).toBe("~0.003600 DASH");
  });

  it("shows distribution fees in breakdown when distributions are enabled", async () => {
    const user = userEvent.setup();
    const dist = createDefaultDistributionState();
    dist.perpetual.enabled = true;
    dist.preProgrammed.enabled = true;
    render(
      <ReviewStep
        {...defaultProps({ onReset, distribution: dist })}
      />,
    );
    await user.click(screen.getByTestId("review-calculate-fee-button"));

    const breakdown = screen.getByTestId("fee-breakdown");
    expect(
      within(breakdown).getByText("Perpetual distribution"),
    ).toBeInTheDocument();
    expect(
      within(breakdown).getByText("Pre-programmed distribution"),
    ).toBeInTheDocument();
  });

  it("shows keyword fees in breakdown when keywords are set", async () => {
    const user = userEvent.setup();
    const kw = createDefaultKeywordsState();
    kw.keywords = ["gaming", "reward"];
    render(
      <ReviewStep {...defaultProps({ onReset, keywords: kw })} />,
    );
    await user.click(screen.getByTestId("review-calculate-fee-button"));

    const breakdown = screen.getByTestId("fee-breakdown");
    expect(
      within(breakdown).getByText("Token keywords (2)"),
    ).toBeInTheDocument();
  });

  it("closes fee dialog via Close button", async () => {
    const user = userEvent.setup();
    render(<ReviewStep {...defaultProps({ onReset })} />);
    await user.click(screen.getByTestId("review-calculate-fee-button"));
    expect(screen.getByTestId("fee-dialog")).toBeInTheDocument();

    await user.click(screen.getByTestId("fee-dialog-close"));
    expect(screen.queryByTestId("fee-dialog")).not.toBeInTheDocument();
  });
});
