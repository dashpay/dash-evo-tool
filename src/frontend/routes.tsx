import { useEffect } from "react";
import {
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  useNavigate,
} from "@tanstack/react-router";
import { AppLayout } from "./AppLayout";
import { WelcomeScreen } from "@/screens/WelcomeScreen";
import { NetworkChooserScreen } from "@/screens/NetworkChooserScreen";
import { WalletsScreen } from "@/screens/WalletsScreen";
import { CreateWalletScreen } from "@/screens/CreateWalletScreen";
import { ImportWalletScreen } from "@/screens/ImportWalletScreen";
import { SendScreen } from "@/screens/SendScreen";
import { SingleKeySendScreen } from "@/screens/SingleKeySendScreen";
import { CreateAssetLockScreen } from "@/screens/CreateAssetLockScreen";
import { AssetLockDetailScreen } from "@/screens/AssetLockDetailScreen";
import { IdentitiesScreen } from "@/screens/IdentitiesScreen";
import { commands } from "@/bindings";
import { DpnsActiveContestsScreen } from "@/screens/DpnsActiveContestsScreen";
import { DpnsPastContestsScreen } from "@/screens/DpnsPastContestsScreen";
import { DpnsOwnedNamesScreen } from "@/screens/DpnsOwnedNamesScreen";
import { DpnsScheduledVotesScreen } from "@/screens/DpnsScheduledVotesScreen";
import { DpnsRegisterNameScreen } from "@/screens/DpnsRegisterNameScreen";
import { ToolsScreen } from "@/screens/ToolsScreen";
import { PlatformInfoScreen } from "@/screens/PlatformInfoScreen";
import { AddressBalanceScreen } from "@/screens/AddressBalanceScreen";
import { ContractVisualizerScreen } from "@/screens/ContractVisualizerScreen";

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

// Root route — just renders children (either welcome or app shell)
const rootRoute = createRootRoute({
  component: Outlet,
});

// Welcome route — full-page, no sidebar or chrome
const welcomeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/welcome",
  component: WelcomeScreen,
});

// App layout route — wraps all authenticated/main routes with sidebar + top bar
const appLayoutRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "app",
  component: AppLayout,
});

// Index route — checks onboarding, redirects accordingly
function IndexRedirect() {
  const navigate = useNavigate();

  useEffect(() => {
    commands
      .settingsGet()
      .then((result) => {
        if (result.status === "ok" && !result.data.onboardingCompleted) {
          navigate({ to: "/welcome", replace: true });
        } else {
          navigate({ to: "/identities", replace: true });
        }
      })
      .catch(() => {
        // Backend not available (browser-only mode) — skip onboarding
        navigate({ to: "/identities", replace: true });
      });
  }, [navigate]);

  return null;
}

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: IndexRedirect,
});

// === Main section routes (children of appLayoutRoute) ===

const dashpayRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
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
  getParentRoute: () => appLayoutRoute,
  path: "/identities",
  component: IdentitiesScreen,
});

const contractsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
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
  component: DpnsActiveContestsScreen,
});

const contractsDpnsPastRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-past",
  component: DpnsPastContestsScreen,
});

const contractsDpnsOwnedRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-owned",
  component: DpnsOwnedNamesScreen,
});

const contractsDpnsScheduledRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-scheduled",
  component: DpnsScheduledVotesScreen,
});

const contractsDpnsRegisterRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-register",
  component: DpnsRegisterNameScreen,
});

const tokensRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
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
  getParentRoute: () => appLayoutRoute,
  path: "/wallets",
  component: () => <Outlet />,
});

const walletsIndexRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/",
  component: WalletsScreen,
});

const walletsCreateRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/create",
  component: CreateWalletScreen,
});

const walletsImportRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/import",
  component: ImportWalletScreen,
});

const walletsSendRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/send/$type",
  component: SendScreen,
});

const walletsSingleKeySendRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/send-single-key",
  component: SingleKeySendScreen,
});

const walletsCreateAssetLockRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/asset-locks/create",
  component: CreateAssetLockScreen,
});

const walletsAssetLockDetailRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/asset-locks/$txid",
  component: AssetLockDetailScreen,
});

const toolsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/tools",
  component: () => <Outlet />,
});

const toolsIndexRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/",
  component: ToolsScreen,
});

const toolsPlatformInfoRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/platform-info",
  component: PlatformInfoScreen,
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
  component: ContractVisualizerScreen,
});

const toolsGroveStarkRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/grovestark",
  component: () => <PlaceholderScreen title="GroveSTARK" />,
});

const toolsAddressBalanceRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/address-balance",
  component: AddressBalanceScreen,
});

const settingsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings",
  component: NetworkChooserScreen,
});

// Build the route tree
const routeTree = rootRoute.addChildren([
  indexRoute,
  welcomeRoute,
  appLayoutRoute.addChildren([
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
      contractsDpnsRegisterRoute,
    ]),
    tokensRoute.addChildren([
      tokensIndexRoute,
      tokensSearchRoute,
      tokensCreatorRoute,
    ]),
    walletsRoute.addChildren([
      walletsIndexRoute,
      walletsCreateRoute,
      walletsImportRoute,
      walletsSendRoute,
      walletsSingleKeySendRoute,
      walletsCreateAssetLockRoute,
      walletsAssetLockDetailRoute,
    ]),
    toolsRoute.addChildren([
      toolsIndexRoute,
      toolsPlatformInfoRoute,
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
  ]),
]);

export const router = createRouter({ routeTree });

// Register the router for type safety
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
