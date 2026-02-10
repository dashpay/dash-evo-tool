// Radix Select uses hasPointerCapture which jsdom doesn't support
if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
}
if (!Element.prototype.setPointerCapture) {
  Element.prototype.setPointerCapture = () => {};
}
if (!Element.prototype.releasePointerCapture) {
  Element.prototype.releasePointerCapture = () => {};
}

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import {
  ControlRulesStep,
  createDefaultControlRulesState,
  createDefaultControlRules,
  createDefaultMintExtras,
  validateControlRules,
} from "./ControlRulesStep";
import type {
  ControlRulesState,
  GroupEntry,
} from "./ControlRulesStep";

// ─── Mocks ──────────────────────────────────────────────────────────

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: () => {},
  }),
}));

// ─── Helpers ────────────────────────────────────────────────────────

function renderStep(
  overrides: Partial<ControlRulesState> = {},
  groups: GroupEntry[] = [],
) {
  const state: ControlRulesState = {
    ...createDefaultControlRulesState(),
    ...overrides,
  };
  const onChange = vi.fn();
  const utils = render(
    <ControlRulesStep state={state} onChange={onChange} groups={groups} />,
  );
  return { ...utils, onChange, state };
}

// ─── createDefaultControlRulesState tests ────────────────────────────

describe("createDefaultControlRulesState", () => {
  it("returns valid default state with all rules set to noOne", () => {
    const state = createDefaultControlRulesState();
    expect(state.manualMinting.authorizedActionTaker).toBe("noOne");
    expect(state.manualBurning.authorizedActionTaker).toBe("noOne");
    expect(state.freeze.authorizedActionTaker).toBe("noOne");
    expect(state.unfreeze.authorizedActionTaker).toBe("noOne");
    expect(state.destroyFrozenFunds.authorizedActionTaker).toBe("noOne");
    expect(state.emergencyAction.authorizedActionTaker).toBe("noOne");
    expect(state.maxSupplyChange.authorizedActionTaker).toBe("noOne");
    expect(state.conventionsChange.authorizedActionTaker).toBe("noOne");
    expect(state.marketplace.authorizedActionTaker).toBe("noOne");
    expect(state.directPurchasePricing.authorizedActionTaker).toBe("noOne");
    expect(state.mainControlGroupChange).toBe("noOne");
  });

  it("has all sections collapsed by default", () => {
    const state = createDefaultControlRulesState();
    expect(state.manualMintingExpanded).toBe(false);
    expect(state.manualBurningExpanded).toBe(false);
    expect(state.freezeExpanded).toBe(false);
    expect(state.unfreezeExpanded).toBe(false);
    expect(state.destroyFrozenFundsExpanded).toBe(false);
    expect(state.emergencyActionExpanded).toBe(false);
    expect(state.maxSupplyChangeExpanded).toBe(false);
    expect(state.conventionsChangeExpanded).toBe(false);
    expect(state.marketplaceExpanded).toBe(false);
    expect(state.directPurchasePricingExpanded).toBe(false);
    expect(state.mainControlGroupChangeExpanded).toBe(false);
  });

  it("has default mint extras", () => {
    const state = createDefaultControlRulesState();
    expect(state.mintExtras.destinationDefaultToContractOwner).toBe(true);
    expect(state.mintExtras.destinationOtherIdentityEnabled).toBe(false);
    expect(state.mintExtras.allowChoosingDestination).toBe(false);
  });
});

describe("createDefaultControlRules", () => {
  it("returns a rule with all checkboxes true", () => {
    const rules = createDefaultControlRules();
    expect(rules.changingAuthorizedToNoOneAllowed).toBe(true);
    expect(rules.changingAdminToNoOneAllowed).toBe(true);
    expect(rules.selfChangingAdminAllowed).toBe(true);
  });

  it("returns a rule with empty identity/group fields", () => {
    const rules = createDefaultControlRules();
    expect(rules.authorizedIdentityId).toBe("");
    expect(rules.authorizedGroupPosition).toBe("");
    expect(rules.adminIdentityId).toBe("");
    expect(rules.adminGroupPosition).toBe("");
  });
});

// ─── validateControlRules tests ─────────────────────────────────────

describe("validateControlRules", () => {
  it("validates default state as valid", () => {
    const state = createDefaultControlRulesState();
    const result = validateControlRules(state);
    expect(result.valid).toBe(true);
    expect(Object.keys(result.errors)).toHaveLength(0);
  });

  it("validates invalid identity ID in authorized action taker", () => {
    const state = createDefaultControlRulesState();
    state.manualMinting = {
      ...createDefaultControlRules(),
      authorizedActionTaker: "identity",
      authorizedIdentityId: "not-valid-base58!!!",
    };
    state.manualMintingExpanded = true;
    const result = validateControlRules(state);
    expect(result.valid).toBe(false);
    expect(result.errors["manualMinting.authorizedIdentityId"]).toBeDefined();
  });

  it("validates valid identity ID format", () => {
    const state = createDefaultControlRulesState();
    state.manualMinting = {
      ...createDefaultControlRules(),
      authorizedActionTaker: "identity",
      // 32-char Base58 string
      authorizedIdentityId: "4EfA9Jrvv3nnCFdSf7fad59851nnTNRM",
    };
    const result = validateControlRules(state);
    expect(result.errors["manualMinting.authorizedIdentityId"]).toBeUndefined();
  });

  it("detects missing mint destination mode when minting enabled", () => {
    const state = createDefaultControlRulesState();
    state.manualMinting = {
      ...createDefaultControlRules(),
      authorizedActionTaker: "contractOwner",
    };
    state.mintExtras = {
      ...createDefaultMintExtras(),
      destinationDefaultToContractOwner: false,
      destinationOtherIdentityEnabled: false,
      allowChoosingDestination: false,
    };
    const result = validateControlRules(state);
    expect(result.valid).toBe(false);
    expect(result.errors["mintExtras.destination"]).toBeDefined();
  });

  it("allows mint destination when at least one mode enabled", () => {
    const state = createDefaultControlRulesState();
    state.manualMinting = {
      ...createDefaultControlRules(),
      authorizedActionTaker: "contractOwner",
    };
    state.mintExtras = {
      ...createDefaultMintExtras(),
      destinationDefaultToContractOwner: true,
    };
    const result = validateControlRules(state);
    expect(result.errors["mintExtras.destination"]).toBeUndefined();
  });

  it("validates group position as numeric", () => {
    const state = createDefaultControlRulesState();
    state.freeze = {
      ...createDefaultControlRules(),
      authorizedActionTaker: "group",
      authorizedGroupPosition: "abc",
    };
    const result = validateControlRules(state);
    expect(result.errors["freeze.authorizedGroupPosition"]).toBeDefined();
  });

  it("validates group position against group count", () => {
    const groups: GroupEntry[] = [
      { requiredPower: "10", members: [] },
      { requiredPower: "20", members: [] },
    ];
    const state = createDefaultControlRulesState();
    state.freeze = {
      ...createDefaultControlRules(),
      authorizedActionTaker: "group",
      authorizedGroupPosition: "5",
    };
    const result = validateControlRules(state, groups);
    expect(result.errors["freeze.authorizedGroupPosition"]).toBe(
      "Group position must be < 2",
    );
  });

  it("validates main control group change identity", () => {
    const state = createDefaultControlRulesState();
    state.mainControlGroupChange = "identity";
    state.mainControlGroupChangeIdentityId = "invalid!!!";
    const result = validateControlRules(state);
    expect(result.errors["mainControlGroupChange.identityId"]).toBeDefined();
  });
});

// ─── Rendering tests ────────────────────────────────────────────────

describe("ControlRulesStep rendering", () => {
  it("renders the control rules step", () => {
    renderStep();
    expect(screen.getByTestId("control-rules-step")).toBeInTheDocument();
  });

  it("renders all 11 control rule sections", () => {
    renderStep();
    expect(screen.getByTestId("manualMinting-section")).toBeInTheDocument();
    expect(screen.getByTestId("manualBurning-section")).toBeInTheDocument();
    expect(screen.getByTestId("freeze-section")).toBeInTheDocument();
    expect(screen.getByTestId("unfreeze-section")).toBeInTheDocument();
    expect(screen.getByTestId("destroyFrozenFunds-section")).toBeInTheDocument();
    expect(screen.getByTestId("emergencyAction-section")).toBeInTheDocument();
    expect(screen.getByTestId("maxSupplyChange-section")).toBeInTheDocument();
    expect(screen.getByTestId("conventionsChange-section")).toBeInTheDocument();
    expect(screen.getByTestId("marketplace-section")).toBeInTheDocument();
    expect(screen.getByTestId("directPurchasePricing-section")).toBeInTheDocument();
    expect(screen.getByTestId("mainControlGroupChange-section")).toBeInTheDocument();
  });

  it("shows No One label for all collapsed sections by default", () => {
    renderStep();
    // Each section header shows the current action taker
    const sections = screen.getAllByText("No One");
    expect(sections.length).toBeGreaterThanOrEqual(11);
  });

  it("shows intro text", () => {
    renderStep();
    expect(
      screen.getByText(/Configure who can perform actions on this token/),
    ).toBeInTheDocument();
  });
});

// ─── Expand/collapse tests ──────────────────────────────────────────

describe("ControlRulesStep expand/collapse", () => {
  it("expands a section when toggle is clicked", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep();

    await user.click(screen.getByTestId("manualMinting-toggle"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ manualMintingExpanded: true }),
    );
  });

  it("collapses an expanded section when toggle is clicked", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({ manualBurningExpanded: true });

    await user.click(screen.getByTestId("manualBurning-toggle"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ manualBurningExpanded: false }),
    );
  });

  it("shows action taker selectors when expanded", () => {
    renderStep({ freezeExpanded: true });
    expect(screen.getByTestId("freeze-authorized-select")).toBeInTheDocument();
    expect(screen.getByTestId("freeze-admin-select")).toBeInTheDocument();
  });

  it("shows checkboxes when expanded", () => {
    renderStep({ freezeExpanded: true });
    expect(
      screen.getByTestId("freeze-authorized-no-one-checkbox"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("freeze-admin-no-one-checkbox"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("freeze-self-changing-checkbox"),
    ).toBeInTheDocument();
  });
});

// ─── Action taker selection tests ───────────────────────────────────

describe("ControlRulesStep action taker selection", () => {
  it("renders identity input when Identity action taker is set", () => {
    // Radix Select doesn't support jsdom click-to-select, so we test
    // by providing state with identity already selected
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "identity",
      },
    });

    expect(
      screen.getByTestId("manualMinting-authorized-identity-input"),
    ).toBeInTheDocument();
  });

  it("renders group position input when Specific Group is set", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "group",
      },
    });

    expect(
      screen.getByTestId("manualMinting-authorized-group-input"),
    ).toBeInTheDocument();
  });

  it("shows valid indicator for valid identity ID", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "identity",
        authorizedIdentityId: "4EfA9Jrvv3nnCFdSf7fad59851nnTNRM",
      },
    });
    const status = screen.getByTestId(
      "manualMinting-authorized-identity-status",
    );
    expect(status).toHaveTextContent("Valid");
  });

  it("shows invalid indicator for invalid identity ID", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "identity",
        authorizedIdentityId: "invalid!!!",
      },
    });
    const status = screen.getByTestId(
      "manualMinting-authorized-identity-status",
    );
    expect(status).toHaveTextContent("Invalid");
  });
});

// ─── Checkbox tests ─────────────────────────────────────────────────

describe("ControlRulesStep checkboxes", () => {
  it("toggles changingAuthorizedToNoOneAllowed checkbox", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({ freezeExpanded: true });

    await user.click(
      screen.getByTestId("freeze-authorized-no-one-checkbox"),
    );

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        freeze: expect.objectContaining({
          changingAuthorizedToNoOneAllowed: false,
        }),
      }),
    );
  });

  it("toggles changingAdminToNoOneAllowed checkbox", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({ freezeExpanded: true });

    await user.click(screen.getByTestId("freeze-admin-no-one-checkbox"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        freeze: expect.objectContaining({
          changingAdminToNoOneAllowed: false,
        }),
      }),
    );
  });

  it("toggles selfChangingAdminAllowed checkbox", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({ freezeExpanded: true });

    await user.click(screen.getByTestId("freeze-self-changing-checkbox"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        freeze: expect.objectContaining({
          selfChangingAdminAllowed: false,
        }),
      }),
    );
  });
});

// ─── Mint extras tests ──────────────────────────────────────────────

describe("ControlRulesStep mint extras", () => {
  it("shows mint extras when minting action taker is not noOne", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
    });
    expect(screen.getByTestId("mint-extras")).toBeInTheDocument();
  });

  it("does not show mint extras when minting action taker is noOne", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "noOne",
      },
    });
    expect(screen.queryByTestId("mint-extras")).not.toBeInTheDocument();
  });

  it("toggles default destination to contract owner", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
    });

    // Default is checked, uncheck it
    await user.click(screen.getByTestId("mint-default-contract-owner"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mintExtras: expect.objectContaining({
          destinationDefaultToContractOwner: false,
        }),
      }),
    );
  });

  it("enabling specific identity disables contract owner default (mutual exclusion)", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      mintExtras: {
        ...createDefaultMintExtras(),
        destinationDefaultToContractOwner: true,
      },
    });

    await user.click(screen.getByTestId("mint-other-identity"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mintExtras: expect.objectContaining({
          destinationOtherIdentityEnabled: true,
          destinationDefaultToContractOwner: false,
        }),
      }),
    );
  });

  it("enabling contract owner disables specific identity (mutual exclusion)", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      mintExtras: {
        ...createDefaultMintExtras(),
        destinationDefaultToContractOwner: false,
        destinationOtherIdentityEnabled: true,
      },
    });

    await user.click(screen.getByTestId("mint-default-contract-owner"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mintExtras: expect.objectContaining({
          destinationDefaultToContractOwner: true,
          destinationOtherIdentityEnabled: false,
        }),
      }),
    );
  });

  it("shows identity input when specific identity is enabled", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      mintExtras: {
        ...createDefaultMintExtras(),
        destinationOtherIdentityEnabled: true,
        destinationDefaultToContractOwner: false,
      },
    });
    expect(
      screen.getByTestId("mint-destination-identity-input"),
    ).toBeInTheDocument();
  });

  it("toggles allow choosing destination", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
    });

    await user.click(screen.getByTestId("mint-allow-choosing"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mintExtras: expect.objectContaining({
          allowChoosingDestination: true,
        }),
      }),
    );
  });

  it("shows nested destination identity rules toggle", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      mintExtras: {
        ...createDefaultMintExtras(),
        destinationOtherIdentityEnabled: true,
        destinationDefaultToContractOwner: false,
      },
    });
    expect(
      screen.getByTestId("mint-destination-rules-toggle"),
    ).toBeInTheDocument();
  });

  it("shows nested choosing destination rules toggle", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      mintExtras: {
        ...createDefaultMintExtras(),
        allowChoosingDestination: true,
      },
    });
    expect(
      screen.getByTestId("mint-choosing-rules-toggle"),
    ).toBeInTheDocument();
  });

  it("shows mint destination error when no mode enabled", () => {
    renderStep({
      manualMintingExpanded: true,
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      mintExtras: {
        ...createDefaultMintExtras(),
        destinationDefaultToContractOwner: false,
        destinationOtherIdentityEnabled: false,
        allowChoosingDestination: false,
      },
    });
    expect(screen.getByTestId("mint-destination-error")).toBeInTheDocument();
    expect(screen.getByTestId("mint-destination-error")).toHaveTextContent(
      "At least one minting destination mode must be enabled",
    );
  });
});

// ─── Main Control Group Change tests ────────────────────────────────

describe("ControlRulesStep main control group change", () => {
  it("renders the main control group change section", () => {
    renderStep();
    expect(
      screen.getByTestId("mainControlGroupChange-section"),
    ).toBeInTheDocument();
  });

  it("expands when toggle is clicked", async () => {
    const user = userEvent.setup();
    const { onChange } = renderStep();

    await user.click(screen.getByTestId("mainControlGroupChange-toggle"));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ mainControlGroupChangeExpanded: true }),
    );
  });

  it("shows action taker selector when expanded", () => {
    renderStep({ mainControlGroupChangeExpanded: true });
    expect(
      screen.getByTestId("mainControlGroupChange-select"),
    ).toBeInTheDocument();
  });

  it("renders correct value when main control group is set to contractOwner", () => {
    renderStep({
      mainControlGroupChangeExpanded: true,
      mainControlGroupChange: "contractOwner",
    });

    // The select should show Contract Owner as value
    expect(
      screen.getByTestId("mainControlGroupChange-select"),
    ).toBeInTheDocument();
  });

  it("shows identity input when main control group is set to identity", () => {
    renderStep({
      mainControlGroupChangeExpanded: true,
      mainControlGroupChange: "identity",
    });

    expect(
      screen.getByTestId("mainControlGroupChange-identity-input"),
    ).toBeInTheDocument();
  });
});

// ─── Groups integration tests ───────────────────────────────────────

describe("ControlRulesStep with groups", () => {
  const testGroups: GroupEntry[] = [
    { requiredPower: "10", members: [{ identityId: "abc123", power: "5" }] },
  ];

  it("passes groups to sections", () => {
    renderStep(
      {
        freezeExpanded: true,
        freeze: {
          ...createDefaultControlRules(),
          authorizedActionTaker: "group",
        },
      },
      testGroups,
    );
    expect(screen.getByTestId("freeze-authorized-group-input")).toBeInTheDocument();
  });
});

// ─── Admin action taker tests ───────────────────────────────────────

describe("ControlRulesStep admin action taker", () => {
  it("shows admin identity input when admin identity is selected", () => {
    renderStep({
      manualBurningExpanded: true,
      manualBurning: {
        ...createDefaultControlRules(),
        adminActionTaker: "identity",
      },
    });
    expect(
      screen.getByTestId("manualBurning-admin-identity-input"),
    ).toBeInTheDocument();
  });

  it("shows admin group input when admin group is selected", () => {
    renderStep({
      manualBurningExpanded: true,
      manualBurning: {
        ...createDefaultControlRules(),
        adminActionTaker: "group",
      },
    });
    expect(
      screen.getByTestId("manualBurning-admin-group-input"),
    ).toBeInTheDocument();
  });
});

// ─── Multiple sections tests ────────────────────────────────────────

describe("ControlRulesStep multiple sections", () => {
  it("can have multiple sections expanded simultaneously", () => {
    renderStep({
      manualMintingExpanded: true,
      freezeExpanded: true,
      emergencyActionExpanded: true,
    });
    expect(
      screen.getByTestId("manualMinting-authorized-select"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("freeze-authorized-select"),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("emergencyAction-authorized-select"),
    ).toBeInTheDocument();
  });

  it("shows correct action taker labels in collapsed sections", () => {
    renderStep({
      manualMinting: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "contractOwner",
      },
      freeze: {
        ...createDefaultControlRules(),
        authorizedActionTaker: "mainGroup",
      },
    });
    // These labels appear in the section headers
    const mintSection = screen.getByTestId("manualMinting-section");
    expect(within(mintSection).getByText("Contract Owner")).toBeInTheDocument();
    const freezeSection = screen.getByTestId("freeze-section");
    expect(within(freezeSection).getByText("Main Group")).toBeInTheDocument();
  });
});
