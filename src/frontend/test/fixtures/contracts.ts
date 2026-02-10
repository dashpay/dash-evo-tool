/**
 * Test fixture factories for contract and document DTOs.
 *
 * Usage:
 *   import { createMockContract, createMockDocument } from "@/test/fixtures";
 *
 *   const contract = createMockContract({ alias: "DPNS" });
 *   const doc = createMockDocument({ documentType: "domain" });
 */

import type {
  DataContractDto,
  ContractSummaryDto,
  ContractBoundsDto,
  WhereClauseDto,
  OrderByClauseDto,
  JsonValue,
} from "@/bindings";
import type {
  DocumentEntry,
  DocumentPageEntry,
} from "@/stores/documentStore";

// ─── Contract factories ────────────────────────────────────────────

export function createMockContract(
  overrides?: Partial<DataContractDto>,
): DataContractDto {
  return {
    id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    ownerId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    alias: "DPNS",
    version: 1,
    documentTypeNames: ["domain", "preorder"],
    tokenCount: 0,
    schemaJson: {
      domain: {
        type: "object",
        properties: {
          label: { type: "string", minLength: 3, maxLength: 63 },
          normalizedLabel: { type: "string" },
          normalizedParentDomainName: { type: "string" },
          records: {
            type: "object",
            properties: {
              dashUniqueIdentityId: { type: "string" },
            },
          },
        },
        required: ["label", "normalizedLabel", "normalizedParentDomainName"],
      },
    } as JsonValue,
    ...overrides,
  };
}

export function createMockContractSummary(
  overrides?: Partial<ContractSummaryDto>,
): ContractSummaryDto {
  return {
    id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "DPNS",
    documentTypeCount: 2,
    tokenCount: 0,
    ...overrides,
  };
}

export function createMockDocumentType(): {
  name: string;
  schema: JsonValue;
  indexes: Array<{ name: string; properties: string[] }>;
} {
  return {
    name: "domain",
    schema: {
      type: "object",
      properties: {
        label: { type: "string", minLength: 3, maxLength: 63 },
        normalizedLabel: { type: "string" },
        normalizedParentDomainName: { type: "string" },
        records: {
          type: "object",
          properties: {
            dashUniqueIdentityId: { type: "string" },
          },
        },
      },
    },
    indexes: [
      {
        name: "parentNameAndLabel",
        properties: ["normalizedParentDomainName", "normalizedLabel"],
      },
    ],
  };
}

// ─── Document factories ────────────────────────────────────────────

export function createMockDocument(
  overrides?: Partial<DocumentEntry>,
): DocumentEntry {
  return {
    id: "BZkLq39rhYNtwpmmFhHjuNXMZq5SnMf39DpE9miFDxpk",
    ownerId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    documentType: "domain",
    data: {
      label: "alice",
      normalizedLabel: "alice",
      normalizedParentDomainName: "dash",
      records: {
        dashUniqueIdentityId:
          "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
      },
    } as JsonValue,
    revision: 1,
    createdAt: 1707500000,
    updatedAt: 1707500000,
    transferredAt: null,
    ...overrides,
  };
}

export function createMockDocumentPage(
  overrides?: Partial<DocumentPageEntry>,
): DocumentPageEntry {
  return {
    id: "BZkLq39rhYNtwpmmFhHjuNXMZq5SnMf39DpE9miFDxpk",
    document: createMockDocument(),
    ...overrides,
  };
}

// ─── Query clause factories ────────────────────────────────────────

export function createMockWhereClause(
  overrides?: Partial<WhereClauseDto>,
): WhereClauseDto {
  return {
    field: "normalizedLabel",
    operator: "==",
    value: "alice" as JsonValue,
    ...overrides,
  };
}

export function createMockOrderByClause(
  overrides?: Partial<OrderByClauseDto>,
): OrderByClauseDto {
  return {
    field: "normalizedLabel",
    direction: "asc",
    ...overrides,
  };
}

// ─── Contract bounds factory ───────────────────────────────────────

export function createMockContractBoundsForContract(
  contractId?: string,
): ContractBoundsDto {
  return {
    type: "singleContract",
    contractId:
      contractId ?? "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
  } as ContractBoundsDto;
}

export function createMockContractBoundsForDocType(
  contractId?: string,
  documentTypeName?: string,
): ContractBoundsDto {
  return {
    type: "singleContractDocumentType",
    contractId:
      contractId ?? "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    documentTypeName: documentTypeName ?? "domain",
  } as ContractBoundsDto;
}
