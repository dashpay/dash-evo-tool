import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Separator } from "@/components/ui/separator";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Island, PageHeader } from "@/components/layout";
import {
  EmptyState,
  LoadingSkeleton,
  SkeletonGroup,
  LoadingSpinner,
  ProgressBar,
} from "@/components/feedback";
import { useTheme } from "@/components/theme";
import { ThemeToggle } from "@/components/theme";
import { Inbox, Wallet, Shield, Coins, Zap } from "lucide-react";

/**
 * Design system showcase page. Demonstrates all themed base components
 * with Dash brand colors in both light and dark modes.
 *
 * This page serves as a living style guide and visual regression reference.
 */
export function DesignSystem() {
  const { resolvedTheme } = useTheme();

  return (
    <div className="min-h-screen bg-background p-8" data-testid="design-system">
      <div className="mx-auto max-w-5xl space-y-8">
        <PageHeader
          title="Design System"
          subtitle={`Current theme: ${resolvedTheme}`}
          breadcrumbs={[{ label: "Home" }, { label: "Design System" }]}
          actions={<ThemeToggle />}
        />

        {/* Colors */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Colors</h2>
          <div className="space-y-4">
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Brand Colors
              </h3>
              <div className="flex flex-wrap gap-2">
                <ColorSwatch name="Dash Blue" className="bg-dash-blue" />
                <ColorSwatch
                  name="Deep Blue"
                  className="bg-dash-deep-blue"
                />
                <ColorSwatch
                  name="Midnight"
                  className="bg-dash-midnight"
                />
              </div>
            </div>
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Semantic Colors
              </h3>
              <div className="flex flex-wrap gap-2">
                <ColorSwatch name="Primary" className="bg-primary" />
                <ColorSwatch name="Destructive" className="bg-destructive" />
                <ColorSwatch name="Success" className="bg-success" />
                <ColorSwatch name="Warning" className="bg-warning" />
                <ColorSwatch name="Info" className="bg-info" />
              </div>
            </div>
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Network Accents
              </h3>
              <div className="flex flex-wrap gap-2">
                <ColorSwatch name="Mainnet" className="bg-network-mainnet" />
                <ColorSwatch name="Testnet" className="bg-network-testnet" />
                <ColorSwatch name="Devnet" className="bg-network-devnet" />
                <ColorSwatch name="Local" className="bg-network-local" />
              </div>
            </div>
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Surface Colors
              </h3>
              <div className="flex flex-wrap gap-2">
                <ColorSwatch
                  name="Background"
                  className="bg-background border"
                  textDark
                />
                <ColorSwatch
                  name="Card"
                  className="bg-card border"
                  textDark
                />
                <ColorSwatch name="Muted" className="bg-muted border" textDark />
                <ColorSwatch
                  name="Accent"
                  className="bg-accent border"
                  textDark
                />
                <ColorSwatch
                  name="Secondary"
                  className="bg-secondary border"
                  textDark
                />
              </div>
            </div>
          </div>
        </Island>

        {/* Buttons */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Buttons</h2>
          <div className="space-y-4">
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Variants
              </h3>
              <div className="flex flex-wrap gap-2">
                <Button>Primary</Button>
                <Button variant="secondary">Secondary</Button>
                <Button variant="outline">Outline</Button>
                <Button variant="ghost">Ghost</Button>
                <Button variant="link">Link</Button>
                <Button variant="destructive">Destructive</Button>
              </div>
            </div>
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Sizes
              </h3>
              <div className="flex flex-wrap items-center gap-2">
                <Button size="xs">Extra Small</Button>
                <Button size="sm">Small</Button>
                <Button size="default">Default</Button>
                <Button size="lg">Large</Button>
              </div>
            </div>
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                States
              </h3>
              <div className="flex flex-wrap gap-2">
                <Button disabled>Disabled</Button>
                <Button>
                  <Zap /> With Icon
                </Button>
              </div>
            </div>
          </div>
        </Island>

        {/* Inputs */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Inputs</h2>
          <div className="grid max-w-md gap-4">
            <Input placeholder="Default input" />
            <Input placeholder="Disabled input" disabled />
            <Input type="password" placeholder="Password input" />
            <Input
              placeholder="Invalid input"
              aria-invalid="true"
              defaultValue="bad value"
            />
          </div>
        </Island>

        {/* Badges */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Badges</h2>
          <div className="flex flex-wrap gap-2">
            <Badge>Default</Badge>
            <Badge variant="secondary">Secondary</Badge>
            <Badge variant="outline">Outline</Badge>
            <Badge variant="destructive">Destructive</Badge>
            <span className="network-badge bg-network-mainnet">Mainnet</span>
            <span className="network-badge bg-network-testnet">Testnet</span>
            <span className="network-badge bg-network-devnet">Devnet</span>
            <span className="network-badge bg-network-local">Local</span>
          </div>
        </Island>

        {/* Cards */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Cards</h2>
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle>Wallet Balance</CardTitle>
                <CardDescription>Total across all addresses</CardDescription>
              </CardHeader>
              <CardContent>
                <p className="text-2xl font-bold">1.234 DASH</p>
              </CardContent>
              <CardFooter>
                <Button size="sm">Send</Button>
              </CardFooter>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>Identity Status</CardTitle>
                <CardDescription>Platform identity info</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex items-center gap-2">
                  <span className="connection-dot-connected" />
                  <span className="text-sm">Registered on Testnet</span>
                </div>
              </CardContent>
              <CardFooter>
                <Button variant="outline" size="sm">
                  View Keys
                </Button>
              </CardFooter>
            </Card>
          </div>
        </Island>

        {/* Tabs */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Tabs</h2>
          <Tabs defaultValue="active" data-testid="tabs-demo">
            <TabsList>
              <TabsTrigger value="active">Active Contests</TabsTrigger>
              <TabsTrigger value="past">Past Contests</TabsTrigger>
              <TabsTrigger value="owned">My Usernames</TabsTrigger>
            </TabsList>
            <TabsContent value="active" className="pt-4">
              <p className="text-sm text-muted-foreground">
                Active DPNS contest entries would appear here.
              </p>
            </TabsContent>
            <TabsContent value="past" className="pt-4">
              <p className="text-sm text-muted-foreground">
                Historical contest results.
              </p>
            </TabsContent>
            <TabsContent value="owned" className="pt-4">
              <p className="text-sm text-muted-foreground">
                Your registered DPNS names.
              </p>
            </TabsContent>
          </Tabs>
        </Island>

        {/* Table */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Table</h2>
          <Table>
            <TableCaption>Sample identity list</TableCaption>
            <TableHeader>
              <TableRow>
                <TableHead>Alias</TableHead>
                <TableHead>Identity ID</TableHead>
                <TableHead>Type</TableHead>
                <TableHead className="text-right">Balance</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow>
                <TableCell className="font-medium">alice</TableCell>
                <TableCell className="font-mono text-xs">
                  4EfA9...k2Xp
                </TableCell>
                <TableCell>
                  <Badge variant="outline">User</Badge>
                </TableCell>
                <TableCell className="text-right">0.500 DASH</TableCell>
              </TableRow>
              <TableRow>
                <TableCell className="font-medium">bob</TableCell>
                <TableCell className="font-mono text-xs">
                  7BcD2...m9Yq
                </TableCell>
                <TableCell>
                  <Badge variant="outline">User</Badge>
                </TableCell>
                <TableCell className="text-right">1.234 DASH</TableCell>
              </TableRow>
              <TableRow>
                <TableCell className="font-medium">masternode-1</TableCell>
                <TableCell className="font-mono text-xs">
                  9FgH4...p3Zr
                </TableCell>
                <TableCell>
                  <Badge variant="secondary">Masternode</Badge>
                </TableCell>
                <TableCell className="text-right">1000.000 DASH</TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </Island>

        <Separator />

        {/* Feedback Components */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Feedback Components</h2>
          <div className="space-y-6">
            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Empty States
              </h3>
              <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
                <EmptyState
                  icon={Inbox}
                  title="No wallets"
                  description="Create a wallet to get started"
                  actionLabel="Create Wallet"
                  onAction={() => {}}
                />
                <EmptyState
                  icon={Shield}
                  title="No identities"
                  description="Register an identity on Platform"
                />
                <EmptyState
                  icon={Coins}
                  title="No tokens"
                  description="Search or create tokens"
                />
              </div>
            </div>

            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Loading States
              </h3>
              <div className="flex flex-wrap items-center gap-6">
                <LoadingSpinner size="sm" label="Loading..." />
                <LoadingSpinner size="md" label="Loading..." />
                <LoadingSpinner size="lg" label="Loading..." />
              </div>
              <div className="mt-4 max-w-md">
                <SkeletonGroup lines={3} />
              </div>
              <div className="mt-4 flex gap-4">
                <LoadingSkeleton width="80px" height="80px" rounded />
                <LoadingSkeleton width="200px" height="20px" />
              </div>
            </div>

            <div>
              <h3 className="mb-2 text-sm font-medium text-muted-foreground">
                Progress Bars
              </h3>
              <div className="max-w-md space-y-3">
                <ProgressBar value={25} max={100} label="SPV Sync" showPercent />
                <ProgressBar
                  value={60}
                  max={100}
                  label="Upload"
                  variant="success"
                  showPercent
                />
                <ProgressBar
                  value={85}
                  max={100}
                  label="Disk Usage"
                  variant="warning"
                  showPercent
                />
                <ProgressBar
                  value={95}
                  max={100}
                  label="Critical"
                  variant="destructive"
                  showPercent
                />
              </div>
            </div>
          </div>
        </Island>

        {/* Connection Indicators */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Connection Indicators</h2>
          <div className="flex items-center gap-6">
            <div className="flex items-center gap-2">
              <span className="connection-dot-connected" />
              <span className="text-sm">Connected</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="connection-dot-disconnected" />
              <span className="text-sm">Disconnected</span>
            </div>
          </div>
        </Island>

        {/* Typography */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Typography</h2>
          <div className="space-y-3">
            <p className="text-4xl font-bold">Display (36px)</p>
            <p className="text-3xl font-bold">Heading 1 (30px)</p>
            <p className="text-2xl font-semibold">Heading 2 (24px)</p>
            <p className="text-xl font-semibold">Heading 3 (20px)</p>
            <p className="text-lg">Large Body (18px)</p>
            <p className="text-base">Body (16px)</p>
            <p className="text-sm text-muted-foreground">
              Secondary (14px)
            </p>
            <p className="text-xs text-muted-foreground">Caption (12px)</p>
            <p className="font-mono text-sm">
              Monospace: 4EfA9Bc2D7k2XpM3nQ8r (JetBrains Mono)
            </p>
          </div>
        </Island>

        {/* Shadows */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Shadows</h2>
          <div className="flex flex-wrap gap-6">
            <div className="rounded-lg bg-card p-6 shadow-sm">shadow-sm</div>
            <div className="rounded-lg bg-card p-6 shadow-md">shadow-md</div>
            <div className="rounded-lg bg-card p-6 shadow-lg">shadow-lg</div>
            <div className="rounded-lg bg-card p-6 shadow-elevated">
              shadow-elevated
            </div>
            <div className="rounded-lg bg-card p-6 shadow-glow">
              shadow-glow
            </div>
          </div>
        </Island>

        {/* Island Component */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Island (Panel)</h2>
          <p className="text-sm text-muted-foreground">
            This entire section is rendered inside an{" "}
            <code className="rounded bg-muted px-1 py-0.5 text-xs">
              &lt;Island&gt;
            </code>{" "}
            component — the standard elevated surface for content areas.
          </p>
        </Island>

        {/* Layout: PageHeader */}
        <Island>
          <h2 className="mb-4 text-xl font-semibold">Page Header</h2>
          <div className="rounded-md border border-dashed p-4">
            <PageHeader
              title="Wallets"
              subtitle="Manage HD and single-key wallets"
              breadcrumbs={[
                { label: "Home", onClick: () => {} },
                { label: "Wallets" },
              ]}
              actions={
                <div className="flex gap-2">
                  <Button size="sm" variant="outline">
                    <Wallet className="size-4" /> Import
                  </Button>
                  <Button size="sm">Create Wallet</Button>
                </div>
              }
            />
          </div>
        </Island>
      </div>
    </div>
  );
}

function ColorSwatch({
  name,
  className,
  textDark,
}: {
  name: string;
  className: string;
  textDark?: boolean;
}) {
  return (
    <div
      className={`flex h-12 w-28 items-center justify-center rounded-md text-xs font-medium ${textDark ? "text-foreground" : "text-white"} ${className}`}
    >
      {name}
    </div>
  );
}

export default DesignSystem;
