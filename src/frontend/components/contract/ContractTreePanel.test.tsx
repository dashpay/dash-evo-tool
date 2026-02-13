import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  ContractTreePanel,
  parseDocumentTypes,
  parseTokens,
} from "./ContractTreePanel";
import type { ContractTreePanelProps, TreeSelection } from "./ContractTreePanel";
import type { ContractSummaryDto, DataContractDto, JsonValue } from "@/bindings";
import { displayId } from "@/lib/utils";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeSummary(
  overrides: Partial<ContractSummaryDto> = {},
): ContractSummaryDto {
  return {
    id: "abc123def456abc123def456abc123def456abc123def456abc123def456abcd",
    alias: null,
    documentTypeCount: 2,
    tokenCount: 0,
    ...overrides,
  };
}

function makeDetail(
  overrides: Partial<DataContractDto> = {},
): DataContractDto {
  return {
    id: "abc123def456abc123def456abc123def456abc123def456abc123def456abcd",
    ownerId: "owner123",
    alias: null,
    version: 1,
    documentTypeNames: ["domain", "preorder"],
    tokenCount: 0,
    schemaJson: {
      documentSchemas: {
        domain: {
          properties: {
            label: { type: "string" },
            normalizedLabel: { type: "string" },
            parentDomainName: { type: "string" },
          },
          indices: [
            {
              name: "parentNameAndLabel",
              unique: true,
              properties: [
                { normalizedLabel: "asc" },
                { parentDomainName: "asc" },
              ],
            },
            {
              name: "identityIndex",
              unique: false,
              properties: [{ normalizedLabel: "asc" }],
            },
          ],
        },
        preorder: {
          properties: {
            saltedDomainHash: { type: "string" },
          },
          indices: [],
        },
      },
    } as JsonValue,
    ...overrides,
  };
}

function makeDetailWithTokens(): DataContractDto {
  return {
    id: "token-contract-id",
    ownerId: "owner456",
    alias: "mytoken",
    version: 1,
    documentTypeNames: [],
    tokenCount: 1,
    schemaJson: {
      tokens: {
        "0": {
          conventions: {
            localizations: {
              en: { name: "MyToken" },
            },
          },
          baseSupply: 1000000,
          maxSupply: 10000000,
        },
      },
    } as JsonValue,
  };
}

const defaultProps: ContractTreePanelProps = {
  contracts: [],
  contractDetails: {},
  selection: null,
  loading: false,
  onExpandContract: vi.fn(),
  onSelectDocumentType: vi.fn(),
  onSelectIndex: vi.fn(),
  onClearSelection: vi.fn(),
  onRemoveContract: vi.fn(),
};

function renderPanel(props: Partial<ContractTreePanelProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <ContractTreePanel {...mergedProps} />
      </TooltipProvider>,
    ),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── parseDocumentTypes tests ──────────────────────────────────────

describe("parseDocumentTypes", () => {
  it("parses document types with properties and indexes", () => {
    const schema = {
      documentSchemas: {
        domain: {
          properties: {
            label: { type: "string" },
            normalizedLabel: { type: "string" },
          },
          indices: [
            {
              name: "byLabel",
              unique: true,
              properties: [{ normalizedLabel: "asc" }],
            },
          ],
        },
      },
    } as JsonValue;

    const result = parseDocumentTypes(schema, ["domain"]);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("domain");
    expect(result[0].properties).toEqual(["label", "normalizedLabel"]);
    expect(result[0].indexes).toHaveLength(1);
    expect(result[0].indexes[0].name).toBe("byLabel");
    expect(result[0].indexes[0].unique).toBe(true);
    expect(result[0].indexes[0].properties).toEqual([
      { field: "normalizedLabel", ascending: true },
    ]);
  });

  it("returns empty properties and indexes for missing schema", () => {
    const result = parseDocumentTypes(null, ["domain"]);
    expect(result).toHaveLength(1);
    expect(result[0].properties).toEqual([]);
    expect(result[0].indexes).toEqual([]);
  });

  it("handles document type not in schema", () => {
    const schema = { documentSchemas: {} } as JsonValue;
    const result = parseDocumentTypes(schema, ["missing"]);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("missing");
    expect(result[0].properties).toEqual([]);
  });

  it("handles document_schemas snake_case key", () => {
    const schema = {
      document_schemas: {
        test: {
          properties: { field1: { type: "string" } },
        },
      },
    } as JsonValue;
    const result = parseDocumentTypes(schema, ["test"]);
    expect(result[0].properties).toEqual(["field1"]);
  });

  it("handles index with descending order", () => {
    const schema = {
      documentSchemas: {
        test: {
          indices: [
            {
              name: "byAge",
              properties: [{ age: "desc" }],
            },
          ],
        },
      },
    } as JsonValue;
    const result = parseDocumentTypes(schema, ["test"]);
    expect(result[0].indexes[0].properties[0].ascending).toBe(false);
  });
});

// ─── parseTokens tests ────────────────────────────────────────────

describe("parseTokens", () => {
  it("parses tokens from object map", () => {
    const schema = {
      tokens: {
        "0": {
          conventions: {
            localizations: { en: { name: "TestCoin" } },
          },
          baseSupply: 1000,
          maxSupply: 5000,
        },
      },
    } as JsonValue;

    const result = parseTokens(schema);
    expect(result).toHaveLength(1);
    expect(result[0].position).toBe("TestCoin");
    expect(result[0].baseSupply).toBe(1000);
    expect(result[0].maxSupply).toBe(5000);
  });

  it("returns empty array for null schema", () => {
    expect(parseTokens(null)).toEqual([]);
  });

  it("returns empty array for schema without tokens", () => {
    expect(parseTokens({ documentSchemas: {} } as JsonValue)).toEqual([]);
  });

  it("uses position label when name not in conventions", () => {
    const schema = {
      tokens: {
        "3": {},
      },
    } as JsonValue;
    const result = parseTokens(schema);
    expect(result).toHaveLength(1);
    expect(result[0].position).toBe("Token 3");
  });

  it("handles null maxSupply", () => {
    const schema = {
      tokens: {
        "0": {
          baseSupply: 100,
          maxSupply: null,
        },
      },
    } as JsonValue;
    const result = parseTokens(schema);
    expect(result[0].maxSupply).toBeNull();
  });

  it("handles snake_case supply fields", () => {
    const schema = {
      tokens: {
        "0": {
          base_supply: 500,
          max_supply: 2000,
        },
      },
    } as JsonValue;
    const result = parseTokens(schema);
    expect(result[0].baseSupply).toBe(500);
    expect(result[0].maxSupply).toBe(2000);
  });
});

// ─── ContractTreePanel rendering tests ─────────────────────────────

describe("ContractTreePanel", () => {
  it("renders the panel with search input", () => {
    renderPanel();
    expect(screen.getByTestId("contract-tree-panel")).toBeInTheDocument();
    expect(screen.getByTestId("contract-search-input")).toBeInTheDocument();
  });

  it("shows loading state", () => {
    renderPanel({ loading: true });
    expect(screen.getByText("Loading contracts...")).toBeInTheDocument();
  });

  it("shows empty state when no contracts", () => {
    renderPanel({ contracts: [] });
    expect(screen.getByTestId("contract-tree-empty")).toBeInTheDocument();
    expect(screen.getByText("No contracts loaded")).toBeInTheDocument();
  });

  it("shows filter empty state when no matches", async () => {
    const contracts = [makeSummary({ alias: "dpns" })];
    const { user } = renderPanel({ contracts });
    await user.type(screen.getByTestId("contract-search-input"), "zzz");
    expect(screen.getByText("No contracts match your filter")).toBeInTheDocument();
  });

  it("renders contract list", () => {
    const contracts = [
      makeSummary({ id: "id1", alias: "dpns" }),
      makeSummary({ id: "id2", alias: "dashpay" }),
    ];
    renderPanel({ contracts });
    expect(screen.getByText("DPNS")).toBeInTheDocument();
    expect(screen.getByText("DashPay")).toBeInTheDocument();
  });

  it("displays system contract pretty names", () => {
    const contracts = [
      makeSummary({ id: "id1", alias: "dpns" }),
      makeSummary({ id: "id2", alias: "keyword_search" }),
      makeSummary({ id: "id3", alias: "token_history" }),
      makeSummary({ id: "id4", alias: "withdrawals" }),
      makeSummary({ id: "id5", alias: "dashpay" }),
    ];
    renderPanel({ contracts });
    expect(screen.getByText("DPNS")).toBeInTheDocument();
    expect(screen.getByText("Keyword Search")).toBeInTheDocument();
    expect(screen.getByText("Token History")).toBeInTheDocument();
    expect(screen.getByText("Withdrawals")).toBeInTheDocument();
    expect(screen.getByText("DashPay")).toBeInTheDocument();
  });

  it("displays custom alias as-is", () => {
    const contracts = [makeSummary({ id: "id1", alias: "My Custom Contract" })];
    renderPanel({ contracts });
    expect(screen.getByText("My Custom Contract")).toBeInTheDocument();
  });

  it("displays truncated ID when no alias", () => {
    const id = "abc123def456abc123def456abc123def456abc123def456abc123def456abcd";
    const contracts = [
      makeSummary({
        id,
        alias: null,
      }),
    ];
    renderPanel({ contracts });
    expect(screen.getByText(displayId(id))).toBeInTheDocument();
  });

  it("filters contracts by search term", async () => {
    const contracts = [
      makeSummary({ id: "id1", alias: "dpns" }),
      makeSummary({ id: "id2", alias: "dashpay" }),
    ];
    const { user } = renderPanel({ contracts });
    const input = screen.getByTestId("contract-search-input");
    await user.type(input, "dpns");
    expect(screen.getByText("DPNS")).toBeInTheDocument();
    expect(screen.queryByText("DashPay")).not.toBeInTheDocument();
  });

  it("filters contracts by ID", async () => {
    const contracts = [
      makeSummary({ id: "abc123", alias: "dpns" }),
      makeSummary({ id: "xyz789", alias: "dashpay" }),
    ];
    const { user } = renderPanel({ contracts });
    const input = screen.getByTestId("contract-search-input");
    await user.type(input, "xyz");
    expect(screen.queryByText("DPNS")).not.toBeInTheDocument();
    expect(screen.getByText("DashPay")).toBeInTheDocument();
  });

  it("shows filter empty state when search matches nothing", async () => {
    const contracts = [makeSummary({ id: "id1", alias: "dpns" })];
    const { user } = renderPanel({ contracts });
    const input = screen.getByTestId("contract-search-input");
    await user.type(input, "nonexistent");
    expect(screen.getByText("No contracts match your filter")).toBeInTheDocument();
  });

  it("clears search filter via clear button", async () => {
    const contracts = [
      makeSummary({ id: "id1", alias: "dpns" }),
      makeSummary({ id: "id2", alias: "dashpay" }),
    ];
    const { user } = renderPanel({ contracts });
    const input = screen.getByTestId("contract-search-input");
    await user.type(input, "dpns");
    expect(screen.queryByText("DashPay")).not.toBeInTheDocument();

    const clearBtn = screen.getByLabelText("Clear search");
    await user.click(clearBtn);
    expect(screen.getByText("DPNS")).toBeInTheDocument();
    expect(screen.getByText("DashPay")).toBeInTheDocument();
  });

  // ─── Expand/collapse ─────────────────────────────────────────────

  it("calls onExpandContract when expanding a contract", async () => {
    const onExpandContract = vi.fn();
    const contracts = [makeSummary({ id: "id1", alias: "dpns" })];
    const { user } = renderPanel({ contracts, onExpandContract });

    const toggle = screen.getByTestId("contract-toggle-id1");
    await user.click(toggle);
    expect(onExpandContract).toHaveBeenCalledWith("id1");
  });

  it("calls onClearSelection when collapsing selected contract", async () => {
    const onClearSelection = vi.fn();
    const onExpandContract = vi.fn();
    const contracts = [makeSummary({ id: "id1", alias: "dpns" })];
    const selection: TreeSelection = { contractId: "id1" };
    const { user } = renderPanel({
      contracts,
      selection,
      onExpandContract,
      onClearSelection,
    });

    // First expand
    await user.click(screen.getByTestId("contract-toggle-id1"));
    // Then collapse
    await user.click(screen.getByTestId("contract-toggle-id1"));
    expect(onClearSelection).toHaveBeenCalled();
  });

  it("shows loading indicator when contract expanded but detail not loaded", async () => {
    const contracts = [makeSummary({ id: "id1", alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: {},
    });

    await user.click(screen.getByTestId("contract-toggle-id1"));
    expect(screen.getByText("Loading...")).toBeInTheDocument();
  });

  // ─── Document types section ──────────────────────────────────────

  it("renders document types section when contract is expanded with detail", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.getByTestId("doc-types-section")).toBeInTheDocument();
    expect(screen.getByTestId("doc-types-toggle")).toBeInTheDocument();
  });

  it("expands document types section and shows types", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));

    expect(screen.getByTestId("doctype-node-domain")).toBeInTheDocument();
    expect(screen.getByTestId("doctype-node-preorder")).toBeInTheDocument();
  });

  it("calls onSelectDocumentType when expanding a doc type", async () => {
    const onSelectDocumentType = vi.fn();
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
      onSelectDocumentType,
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));
    await user.click(screen.getByTestId("doctype-toggle-domain"));

    expect(onSelectDocumentType).toHaveBeenCalledWith(
      detail.id,
      "domain",
      ["label", "normalizedLabel", "parentDomainName"],
    );
  });

  // ─── Indexes ──────────────────────────────────────────────────────

  it("shows indexes when doc type is expanded", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));
    await user.click(screen.getByTestId("doctype-toggle-domain"));

    expect(screen.getByTestId("index-node-parentNameAndLabel")).toBeInTheDocument();
    expect(screen.getByTestId("index-node-identityIndex")).toBeInTheDocument();
  });

  it("shows 'No indexes defined' for types without indexes", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));
    await user.click(screen.getByTestId("doctype-toggle-preorder"));

    expect(screen.getByText("No indexes defined")).toBeInTheDocument();
  });

  it("calls onSelectIndex when expanding an index", async () => {
    const onSelectIndex = vi.fn();
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
      onSelectIndex,
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));
    await user.click(screen.getByTestId("doctype-toggle-domain"));
    await user.click(screen.getByTestId("index-toggle-parentNameAndLabel"));

    expect(onSelectIndex).toHaveBeenCalledWith(
      detail.id,
      "domain",
      expect.objectContaining({
        name: "parentNameAndLabel",
        unique: true,
        properties: [
          { field: "normalizedLabel", ascending: true },
          { field: "parentDomainName", ascending: true },
        ],
      }),
    );
  });

  it("shows index properties when index is expanded", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));
    await user.click(screen.getByTestId("doctype-toggle-domain"));
    await user.click(screen.getByTestId("index-toggle-parentNameAndLabel"));

    expect(screen.getByTestId("index-property-normalizedLabel")).toBeInTheDocument();
    expect(screen.getByTestId("index-property-parentDomainName")).toBeInTheDocument();
    expect(screen.getByText("normalizedLabel (asc)")).toBeInTheDocument();
    expect(screen.getByText("parentDomainName (asc)")).toBeInTheDocument();
  });

  it("shows unique badge on unique indexes", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("doc-types-toggle"));
    await user.click(screen.getByTestId("doctype-toggle-domain"));

    // The parentNameAndLabel index has unique: true
    const indexNode = screen.getByTestId("index-node-parentNameAndLabel");
    expect(within(indexNode).getByText("unique")).toBeInTheDocument();
  });

  // ─── Tokens section ──────────────────────────────────────────────

  it("renders tokens section when contract has tokens", async () => {
    const detail = makeDetailWithTokens();
    const contracts = [
      makeSummary({
        id: detail.id,
        alias: detail.alias,
        tokenCount: 1,
        documentTypeCount: 0,
      }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.getByTestId("tokens-section")).toBeInTheDocument();
  });

  it("expands tokens section and shows token info", async () => {
    const detail = makeDetailWithTokens();
    const contracts = [
      makeSummary({
        id: detail.id,
        alias: detail.alias,
        tokenCount: 1,
        documentTypeCount: 0,
      }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("tokens-toggle"));
    expect(screen.getByTestId("token-node-0")).toBeInTheDocument();
  });

  it("shows token supply details when expanded", async () => {
    const detail = makeDetailWithTokens();
    const contracts = [
      makeSummary({
        id: detail.id,
        alias: detail.alias,
        tokenCount: 1,
        documentTypeCount: 0,
      }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("tokens-toggle"));
    await user.click(screen.getByTestId("token-toggle-0"));

    expect(screen.getByTestId("token-base-supply-0")).toHaveTextContent("Base Supply: 1000000");
    expect(screen.getByTestId("token-max-supply-0")).toHaveTextContent("Max Supply: 10000000");
  });

  // ─── Contract JSON section ───────────────────────────────────────

  it("renders contract JSON section", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.getByTestId("contract-json-section")).toBeInTheDocument();
  });

  it("calls onSelectContractJson when Contract JSON is clicked", async () => {
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const onSelectContractJson = vi.fn();
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
      onSelectContractJson,
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId("contract-json-toggle"));

    expect(onSelectContractJson).toHaveBeenCalledWith(detail.id);
  });

  // ─── Remove contract ─────────────────────────────────────────────

  it("shows remove button for non-system contracts", async () => {
    const detail = makeDetail({ alias: "my-contract" });
    const contracts = [
      makeSummary({ id: detail.id, alias: "my-contract" }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.getByTestId(`contract-remove-${detail.id}`)).toBeInTheDocument();
  });

  it("does not show remove button for system contracts", async () => {
    const detail = makeDetail({ alias: "dpns" });
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.queryByTestId(`contract-remove-${detail.id}`)).not.toBeInTheDocument();
  });

  it("does not show remove button for dashpay system contract", async () => {
    const detail = makeDetail({ alias: "dashpay" });
    const contracts = [makeSummary({ id: detail.id, alias: "dashpay" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.queryByTestId(`contract-remove-${detail.id}`)).not.toBeInTheDocument();
  });

  it("shows confirmation dialog on remove click", async () => {
    const detail = makeDetail({ alias: "user-contract" });
    const contracts = [
      makeSummary({ id: detail.id, alias: "user-contract" }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId(`contract-remove-${detail.id}`));

    expect(screen.getByText("Remove Contract")).toBeInTheDocument();
    expect(
      screen.getByText(
        'Are you sure you want to remove "user-contract"? This will remove it from your local database.',
      ),
    ).toBeInTheDocument();
  });

  it("calls onRemoveContract on confirmation", async () => {
    const onRemoveContract = vi.fn();
    const detail = makeDetail({ alias: "user-contract" });
    const contracts = [
      makeSummary({ id: detail.id, alias: "user-contract" }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
      onRemoveContract,
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId(`contract-remove-${detail.id}`));

    // Click the confirm button in the dialog
    const confirmBtn = screen.getByRole("button", { name: "Remove" });
    await user.click(confirmBtn);

    expect(onRemoveContract).toHaveBeenCalledWith(detail.id);
  });

  it("does not call onRemoveContract on cancellation", async () => {
    const onRemoveContract = vi.fn();
    const detail = makeDetail({ alias: "user-contract" });
    const contracts = [
      makeSummary({ id: detail.id, alias: "user-contract" }),
    ];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
      onRemoveContract,
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    await user.click(screen.getByTestId(`contract-remove-${detail.id}`));

    const cancelBtn = screen.getByRole("button", { name: "Cancel" });
    await user.click(cancelBtn);

    expect(onRemoveContract).not.toHaveBeenCalled();
  });

  // ─── Copy actions ─────────────────────────────────────────────────

  it("renders copy actions menu when contract expanded", async () => {
    const onCopyId = vi.fn();
    const onCopyJson = vi.fn();
    const detail = makeDetail();
    const contracts = [makeSummary({ id: detail.id, alias: "dpns" })];
    const { user } = renderPanel({
      contracts,
      contractDetails: { [detail.id]: detail },
      onCopyId,
      onCopyJson,
    });

    await user.click(screen.getByTestId(`contract-toggle-${detail.id}`));
    expect(screen.getByTestId(`contract-menu-${detail.id}`)).toBeInTheDocument();
  });

  // ─── Selection highlighting ──────────────────────────────────────

  it("highlights selected contract", () => {
    const contracts = [makeSummary({ id: "id1", alias: "dpns" })];
    const selection: TreeSelection = { contractId: "id1" };
    renderPanel({ contracts, selection });

    const toggle = screen.getByTestId("contract-toggle-id1");
    expect(toggle).toHaveClass("text-primary");
  });

  // ─── Accessibility ───────────────────────────────────────────────

  it("has accessible tree role when contracts are loaded", () => {
    renderPanel({ contracts: [makeSummary()] });
    expect(screen.getByRole("tree")).toBeInTheDocument();
  });

  it("omits tree role when no contracts are loaded", () => {
    renderPanel({ contracts: [] });
    expect(screen.queryByRole("tree")).not.toBeInTheDocument();
  });

  it("search input has aria-label", () => {
    renderPanel();
    expect(screen.getByLabelText("Filter contracts")).toBeInTheDocument();
  });
});
