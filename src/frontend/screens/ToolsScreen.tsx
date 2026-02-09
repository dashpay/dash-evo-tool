import { useNavigate } from "@tanstack/react-router";
import { Island } from "@/components/layout/Island";
import { PageHeader } from "@/components/layout/PageHeader";
import {
  Info,
  Wallet,
  ScrollText,
  ArrowRightLeft,
  FileText,
  ShieldCheck,
  FileCode,
  Network,
  Shield,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

interface ToolCard {
  id: string;
  title: string;
  description: string;
  icon: LucideIcon;
  path: string;
}

interface ToolCategory {
  name: string;
  tools: ToolCard[];
}

const toolCategories: ToolCategory[] = [
  {
    name: "Query & Inspection",
    tools: [
      {
        id: "platform-info",
        title: "Platform Info",
        description:
          "Fetch platform data: epoch info, credits, version voting, validators, and withdrawals.",
        icon: Info,
        path: "/tools/platform-info",
      },
      {
        id: "address-balance",
        title: "Address Balance",
        description:
          "Look up the balance and nonce of a platform address.",
        icon: Wallet,
        path: "/tools/address-balance",
      },
      {
        id: "proof-log",
        title: "Proof Log",
        description:
          "Browse and inspect historical proof log entries with sorting, filtering, and detail views.",
        icon: ScrollText,
        path: "/tools/proof-log",
      },
      {
        id: "masternode-list",
        title: "Masternode List Diff",
        description:
          "Inspect masternode list diffs, chain locks, instant locks, and quorum entries.",
        icon: Network,
        path: "/tools/masternode-list",
      },
    ],
  },
  {
    name: "Deserializers",
    tools: [
      {
        id: "transition-visualizer",
        title: "Transition Visualizer",
        description:
          "Deserialize and visualize state transitions. Detect contract IDs and broadcast.",
        icon: ArrowRightLeft,
        path: "/tools/transition-visualizer",
      },
      {
        id: "contract-visualizer",
        title: "Contract Visualizer",
        description:
          "Deserialize and visualize data contracts from hex, base64, or CSV input.",
        icon: FileCode,
        path: "/tools/contract-visualizer",
      },
      {
        id: "document-visualizer",
        title: "Document Visualizer",
        description:
          "Deserialize documents with contract and document type context.",
        icon: FileText,
        path: "/tools/document-visualizer",
      },
      {
        id: "proof-visualizer",
        title: "Proof Visualizer",
        description:
          "Deserialize and inspect GroveDB proof structures from binary data.",
        icon: ShieldCheck,
        path: "/tools/proof-visualizer",
      },
    ],
  },
  {
    name: "Advanced",
    tools: [
      {
        id: "grovestark",
        title: "GroveSTARK",
        description:
          "Generate and verify zero-knowledge proofs for platform documents.",
        icon: Shield,
        path: "/tools/grovestark",
      },
    ],
  },
];

function ToolCardItem({ tool }: { tool: ToolCard }) {
  const navigate = useNavigate();
  const Icon = tool.icon;

  return (
    <button
      type="button"
      onClick={() => navigate({ to: tool.path })}
      className={cn(
        "group flex flex-col items-start gap-3 rounded-lg border bg-card p-5 text-left",
        "transition-all hover:border-primary/30 hover:shadow-md",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
      )}
    >
      <div
        className={cn(
          "flex size-10 items-center justify-center rounded-lg",
          "bg-primary/10 text-primary transition-colors",
          "group-hover:bg-primary/15",
        )}
      >
        <Icon className="size-5" />
      </div>
      <div className="space-y-1">
        <h3 className="font-semibold text-foreground">{tool.title}</h3>
        <p className="text-sm text-muted-foreground leading-relaxed">
          {tool.description}
        </p>
      </div>
    </button>
  );
}

/**
 * Tools landing page with a categorized card grid linking to each sub-tool.
 *
 * Categories:
 * - Query & Inspection: Platform Info, Address Balance, Proof Log, Masternode List Diff
 * - Deserializers: Transition, Contract, Document, Proof Visualizers
 * - Advanced: GroveSTARK
 */
export function ToolsScreen() {
  return (
    <Island className="flex-1 overflow-auto">
      <PageHeader
        title="Tools"
        subtitle="Platform utilities and data inspection tools"
      />

      <div className="mt-6 space-y-8">
        {toolCategories.map((category) => (
          <section key={category.name}>
            <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
              {category.name}
            </h2>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
              {category.tools.map((tool) => (
                <ToolCardItem key={tool.id} tool={tool} />
              ))}
            </div>
          </section>
        ))}
      </div>
    </Island>
  );
}
