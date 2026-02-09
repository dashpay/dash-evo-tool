import {
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  redirect,
} from "@tanstack/react-router";
import { AppLayout } from "./AppLayout";

// Placeholder screen components — each renders a simple page for now,
// to be replaced with full implementations in later phases.
function PlaceholderScreen({ title }: { title: string }) {
  return (
    <div className="flex flex-1 items-center justify-center text-muted-foreground">
      <div className="text-center">
        <h2 className="text-2xl font-bold text-foreground">{title}</h2>
        <p className="mt-2 text-sm">This screen will be implemented in a future phase.</p>
      </div>
    </div>
  );
}

// Root route with the app layout shell
const rootRoute = createRootRoute({
  component: AppLayout,
});

// Index route — redirect to identities by default
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/identities" });
  },
});

// === Main section routes ===

const dashpayRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dashpay",
  component: () => <Outlet />,
});

const dashpayIndexRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/",
  component: () => <PlaceholderScreen title="DashPay" />,
});

const dashpayProfileRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/profile",
  component: () => <PlaceholderScreen title="DashPay Profile" />,
});

const dashpayContactsRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/contacts",
  component: () => <PlaceholderScreen title="DashPay Contacts" />,
});

const dashpayPaymentsRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/payments",
  component: () => <PlaceholderScreen title="DashPay Payments" />,
});

const dashpaySearchRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/search",
  component: () => <PlaceholderScreen title="Profile Search" />,
});

const identitiesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/identities",
  component: () => <PlaceholderScreen title="Identities" />,
});

const contractsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/contracts",
  component: () => <Outlet />,
});

const contractsIndexRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/",
  component: () => <PlaceholderScreen title="Document Query" />,
});

const contractsDpnsActiveRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-active",
  component: () => <PlaceholderScreen title="DPNS Active Contests" />,
});

const contractsDpnsPastRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-past",
  component: () => <PlaceholderScreen title="DPNS Past Contests" />,
});

const contractsDpnsOwnedRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-owned",
  component: () => <PlaceholderScreen title="DPNS Owned Names" />,
});

const contractsDpnsScheduledRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-scheduled",
  component: () => <PlaceholderScreen title="DPNS Scheduled Votes" />,
});

const tokensRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tokens",
  component: () => <Outlet />,
});

const tokensIndexRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/",
  component: () => <PlaceholderScreen title="My Tokens" />,
});

const tokensSearchRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/search",
  component: () => <PlaceholderScreen title="Search Tokens" />,
});

const tokensCreatorRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/creator",
  component: () => <PlaceholderScreen title="Token Creator" />,
});

const walletsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/wallets",
  component: () => <PlaceholderScreen title="Wallets" />,
});

const toolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tools",
  component: () => <Outlet />,
});

const toolsIndexRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/",
  component: () => <PlaceholderScreen title="Platform Info" />,
});

const toolsProofLogRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/proof-log",
  component: () => <PlaceholderScreen title="Proof Log" />,
});

const toolsTransitionRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/transition-visualizer",
  component: () => <PlaceholderScreen title="Transition Visualizer" />,
});

const toolsDocumentRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/document-visualizer",
  component: () => <PlaceholderScreen title="Document Visualizer" />,
});

const toolsProofVisualizerRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/proof-visualizer",
  component: () => <PlaceholderScreen title="Proof Visualizer" />,
});

const toolsMnListRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/masternode-list",
  component: () => <PlaceholderScreen title="Masternode List Diff" />,
});

const toolsContractRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/contract-visualizer",
  component: () => <PlaceholderScreen title="Contract Visualizer" />,
});

const toolsGroveStarkRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/grovestark",
  component: () => <PlaceholderScreen title="GroveSTARK" />,
});

const toolsAddressBalanceRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/address-balance",
  component: () => <PlaceholderScreen title="Address Balance" />,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: () => <PlaceholderScreen title="Settings" />,
});

// Build the route tree
const routeTree = rootRoute.addChildren([
  indexRoute,
  dashpayRoute.addChildren([
    dashpayIndexRoute,
    dashpayProfileRoute,
    dashpayContactsRoute,
    dashpayPaymentsRoute,
    dashpaySearchRoute,
  ]),
  identitiesRoute,
  contractsRoute.addChildren([
    contractsIndexRoute,
    contractsDpnsActiveRoute,
    contractsDpnsPastRoute,
    contractsDpnsOwnedRoute,
    contractsDpnsScheduledRoute,
  ]),
  tokensRoute.addChildren([
    tokensIndexRoute,
    tokensSearchRoute,
    tokensCreatorRoute,
  ]),
  walletsRoute,
  toolsRoute.addChildren([
    toolsIndexRoute,
    toolsProofLogRoute,
    toolsTransitionRoute,
    toolsDocumentRoute,
    toolsProofVisualizerRoute,
    toolsMnListRoute,
    toolsContractRoute,
    toolsGroveStarkRoute,
    toolsAddressBalanceRoute,
  ]),
  settingsRoute,
]);

export const router = createRouter({ routeTree });

// Register the router for type safety
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
