import { lazy, Suspense, useEffect } from "react";
import {
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  useNavigate,
  useParams,
} from "@tanstack/react-router";
import { AppLayout } from "./AppLayout";
import { commands } from "@/bindings";
import { LoadingSpinner } from "@/components/feedback";

// Lazy-loaded screen components — each becomes its own chunk
const WelcomeScreen = lazy(() =>
  import("@/screens/WelcomeScreen").then((m) => ({ default: m.WelcomeScreen })),
);
const NetworkChooserScreen = lazy(() =>
  import("@/screens/NetworkChooserScreen").then((m) => ({
    default: m.NetworkChooserScreen,
  })),
);
const WalletsScreen = lazy(() =>
  import("@/screens/WalletsScreen").then((m) => ({
    default: m.WalletsScreen,
  })),
);
const CreateWalletScreen = lazy(() =>
  import("@/screens/CreateWalletScreen").then((m) => ({
    default: m.CreateWalletScreen,
  })),
);
const ImportWalletScreen = lazy(() =>
  import("@/screens/ImportWalletScreen").then((m) => ({
    default: m.ImportWalletScreen,
  })),
);
const SendScreen = lazy(() =>
  import("@/screens/SendScreen").then((m) => ({ default: m.SendScreen })),
);
const SingleKeySendScreen = lazy(() =>
  import("@/screens/SingleKeySendScreen").then((m) => ({
    default: m.SingleKeySendScreen,
  })),
);
const CreateAssetLockScreen = lazy(() =>
  import("@/screens/CreateAssetLockScreen").then((m) => ({
    default: m.CreateAssetLockScreen,
  })),
);
const AssetLockDetailScreen = lazy(() =>
  import("@/screens/AssetLockDetailScreen").then((m) => ({
    default: m.AssetLockDetailScreen,
  })),
);
const IdentitiesScreen = lazy(() =>
  import("@/screens/IdentitiesScreen").then((m) => ({
    default: m.IdentitiesScreen,
  })),
);
const DpnsActiveContestsScreen = lazy(() =>
  import("@/screens/DpnsActiveContestsScreen").then((m) => ({
    default: m.DpnsActiveContestsScreen,
  })),
);
const DpnsPastContestsScreen = lazy(() =>
  import("@/screens/DpnsPastContestsScreen").then((m) => ({
    default: m.DpnsPastContestsScreen,
  })),
);
const DpnsOwnedNamesScreen = lazy(() =>
  import("@/screens/DpnsOwnedNamesScreen").then((m) => ({
    default: m.DpnsOwnedNamesScreen,
  })),
);
const DpnsScheduledVotesScreen = lazy(() =>
  import("@/screens/DpnsScheduledVotesScreen").then((m) => ({
    default: m.DpnsScheduledVotesScreen,
  })),
);
const DpnsRegisterNameScreen = lazy(() =>
  import("@/screens/DpnsRegisterNameScreen").then((m) => ({
    default: m.DpnsRegisterNameScreen,
  })),
);
const ToolsScreen = lazy(() =>
  import("@/screens/ToolsScreen").then((m) => ({ default: m.ToolsScreen })),
);
const PlatformInfoScreen = lazy(() =>
  import("@/screens/PlatformInfoScreen").then((m) => ({
    default: m.PlatformInfoScreen,
  })),
);
const AddressBalanceScreen = lazy(() =>
  import("@/screens/AddressBalanceScreen").then((m) => ({
    default: m.AddressBalanceScreen,
  })),
);
const ContractVisualizerScreen = lazy(() =>
  import("@/screens/ContractVisualizerScreen").then((m) => ({
    default: m.ContractVisualizerScreen,
  })),
);
const DocumentVisualizerScreen = lazy(() =>
  import("@/screens/DocumentVisualizerScreen").then((m) => ({
    default: m.DocumentVisualizerScreen,
  })),
);
const ProofVisualizerScreen = lazy(() =>
  import("@/screens/ProofVisualizerScreen").then((m) => ({
    default: m.ProofVisualizerScreen,
  })),
);
const TransitionVisualizerScreen = lazy(() =>
  import("@/screens/TransitionVisualizerScreen").then((m) => ({
    default: m.TransitionVisualizerScreen,
  })),
);
const ProofLogScreen = lazy(() =>
  import("@/screens/ProofLogScreen").then((m) => ({
    default: m.ProofLogScreen,
  })),
);
const DocumentQueryScreen = lazy(() =>
  import("@/screens/DocumentQueryScreen").then((m) => ({
    default: m.DocumentQueryScreen,
  })),
);
const AddContractsScreen = lazy(() =>
  import("@/screens/AddContractsScreen").then((m) => ({
    default: m.AddContractsScreen,
  })),
);
const RegisterContractScreen = lazy(() =>
  import("@/screens/RegisterContractScreen").then((m) => ({
    default: m.RegisterContractScreen,
  })),
);
const UpdateContractScreen = lazy(() =>
  import("@/screens/UpdateContractScreen").then((m) => ({
    default: m.UpdateContractScreen,
  })),
);
const DocumentActionScreen = lazy(() =>
  import("@/screens/DocumentActionScreen").then((m) => ({
    default: m.DocumentActionScreen,
  })),
);
const GroupActionsScreen = lazy(() =>
  import("@/screens/GroupActionsScreen").then((m) => ({
    default: m.GroupActionsScreen,
  })),
);
const TokenMyTokensScreen = lazy(() =>
  import("@/screens/TokenMyTokensScreen").then((m) => ({
    default: m.TokenMyTokensScreen,
  })),
);
const TokenSearchScreen = lazy(() =>
  import("@/screens/TokenSearchScreen").then((m) => ({
    default: m.TokenSearchScreen,
  })),
);
const TokenAddByIdScreen = lazy(() =>
  import("@/screens/TokenAddByIdScreen").then((m) => ({
    default: m.TokenAddByIdScreen,
  })),
);
const TokenCreatorScreen = lazy(() =>
  import("@/screens/TokenCreatorScreen").then((m) => ({
    default: m.TokenCreatorScreen,
  })),
);
const TokenTransferScreen = lazy(() =>
  import("@/screens/TokenTransferScreen").then((m) => ({
    default: m.TokenTransferScreen,
  })),
);
const TokenMintScreen = lazy(() =>
  import("@/screens/TokenMintScreen").then((m) => ({
    default: m.TokenMintScreen,
  })),
);
const TokenBurnScreen = lazy(() =>
  import("@/screens/TokenBurnScreen").then((m) => ({
    default: m.TokenBurnScreen,
  })),
);
const TokenFreezeScreen = lazy(() =>
  import("@/screens/TokenFreezeScreen").then((m) => ({
    default: m.TokenFreezeScreen,
  })),
);
const TokenUnfreezeScreen = lazy(() =>
  import("@/screens/TokenUnfreezeScreen").then((m) => ({
    default: m.TokenUnfreezeScreen,
  })),
);
const TokenDestroyFrozenFundsScreen = lazy(() =>
  import("@/screens/TokenDestroyFrozenFundsScreen").then((m) => ({
    default: m.TokenDestroyFrozenFundsScreen,
  })),
);
const TokenPauseScreen = lazy(() =>
  import("@/screens/TokenPauseScreen").then((m) => ({
    default: m.TokenPauseScreen,
  })),
);
const TokenResumeScreen = lazy(() =>
  import("@/screens/TokenResumeScreen").then((m) => ({
    default: m.TokenResumeScreen,
  })),
);
const TokenClaimScreen = lazy(() =>
  import("@/screens/TokenClaimScreen").then((m) => ({
    default: m.TokenClaimScreen,
  })),
);
const TokenViewClaimsScreen = lazy(() =>
  import("@/screens/TokenViewClaimsScreen").then((m) => ({
    default: m.TokenViewClaimsScreen,
  })),
);
const TokenSetPriceScreen = lazy(() =>
  import("@/screens/TokenSetPriceScreen").then((m) => ({
    default: m.TokenSetPriceScreen,
  })),
);
const TokenPurchaseScreen = lazy(() =>
  import("@/screens/TokenPurchaseScreen").then((m) => ({
    default: m.TokenPurchaseScreen,
  })),
);
const TokenUpdateConfigScreen = lazy(() =>
  import("@/screens/TokenUpdateConfigScreen").then((m) => ({
    default: m.TokenUpdateConfigScreen,
  })),
);
const DashPayScreen = lazy(() =>
  import("@/screens/DashPayScreen").then((m) => ({
    default: m.DashPayScreen,
  })),
);
const ProfileScreen = lazy(() =>
  import("@/screens/ProfileScreen").then((m) => ({
    default: m.ProfileScreen,
  })),
);
const ContactsListScreen = lazy(() =>
  import("@/screens/ContactsListScreen").then((m) => ({
    default: m.ContactsListScreen,
  })),
);
const AddContactScreen = lazy(() =>
  import("@/screens/AddContactScreen").then((m) => ({
    default: m.AddContactScreen,
  })),
);
const ContactDetailsScreen = lazy(() =>
  import("@/screens/ContactDetailsScreen").then((m) => ({
    default: m.ContactDetailsScreen,
  })),
);
const ContactProfileViewer = lazy(() =>
  import("@/screens/ContactProfileViewer").then((m) => ({
    default: m.ContactProfileViewer,
  })),
);
const ContactInfoEditorScreen = lazy(() =>
  import("@/screens/ContactInfoEditorScreen").then((m) => ({
    default: m.ContactInfoEditorScreen,
  })),
);
const SendPaymentScreen = lazy(() =>
  import("@/screens/SendPaymentScreen").then((m) => ({
    default: m.SendPaymentScreen,
  })),
);
const PaymentHistoryScreen = lazy(() =>
  import("@/screens/PaymentHistoryScreen").then((m) => ({
    default: m.PaymentHistoryScreen,
  })),
);
const ProfileSearchScreen = lazy(() =>
  import("@/screens/ProfileSearchScreen").then((m) => ({
    default: m.ProfileSearchScreen,
  })),
);
const QRCodeGeneratorScreen = lazy(() =>
  import("@/screens/QRCodeGeneratorScreen").then((m) => ({
    default: m.QRCodeGeneratorScreen,
  })),
);
const QRScannerScreen = lazy(() =>
  import("@/screens/QRScannerScreen").then((m) => ({
    default: m.QRScannerScreen,
  })),
);
const GroveSTARKScreen = lazy(() =>
  import("@/screens/GroveSTARKScreen").then((m) => ({
    default: m.GroveSTARKScreen,
  })),
);
const MasternodeListDiffScreen = lazy(() =>
  import("@/screens/MasternodeListDiffScreen").then((m) => ({
    default: m.MasternodeListDiffScreen,
  })),
);

/** Suspense wrapper for lazy-loaded screen components */
function LazyScreen({ children }: { children: React.ReactNode }) {
  return (
    <Suspense
      fallback={
        <div className="flex h-full items-center justify-center">
          <LoadingSpinner size="lg" />
        </div>
      }
    >
      {children}
    </Suspense>
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
  component: () => (
    <LazyScreen>
      <WelcomeScreen />
    </LazyScreen>
  ),
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
  component: () => (
    <LazyScreen>
      <DashPayScreen />
    </LazyScreen>
  ),
});

const dashpayIndexRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/",
  // DashPayScreen redirects /dashpay → /dashpay/profile
  component: () => null,
});

const dashpayProfileRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/profile",
  component: () => (
    <LazyScreen>
      <ProfileScreen />
    </LazyScreen>
  ),
});

const dashpayContactsRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/contacts",
  component: () => (
    <LazyScreen>
      <ContactsListScreen />
    </LazyScreen>
  ),
});

const dashpayPaymentsRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/payments",
  component: () => (
    <LazyScreen>
      <PaymentHistoryScreen />
    </LazyScreen>
  ),
});

const dashpaySearchRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/search",
  component: () => (
    <LazyScreen>
      <ProfileSearchScreen />
    </LazyScreen>
  ),
});

const dashpayAddContactRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/add-contact",
  component: () => (
    <LazyScreen>
      <AddContactScreen />
    </LazyScreen>
  ),
});

function ContactDetailsWrapper() {
  const { contactId } = useParams({ strict: false }) as {
    contactId: string;
  };
  return (
    <LazyScreen>
      <ContactDetailsScreen contactId={contactId} />
    </LazyScreen>
  );
}

const dashpayContactDetailsRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/contact-details/$contactId",
  component: ContactDetailsWrapper,
});

function ContactProfileViewerWrapper() {
  const { contactId } = useParams({ strict: false }) as {
    contactId: string;
  };
  return (
    <LazyScreen>
      <ContactProfileViewer contactId={contactId} />
    </LazyScreen>
  );
}

const dashpayContactProfileRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/contact-profile/$contactId",
  component: ContactProfileViewerWrapper,
});

function ContactInfoEditorWrapper() {
  const { contactId } = useParams({ strict: false }) as {
    contactId: string;
  };
  return (
    <LazyScreen>
      <ContactInfoEditorScreen contactId={contactId} />
    </LazyScreen>
  );
}

const dashpayContactInfoEditorRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/contact-info-editor/$contactId",
  component: ContactInfoEditorWrapper,
});

function SendPaymentWrapper() {
  const { contactId } = useParams({ strict: false }) as {
    contactId: string;
  };
  return (
    <LazyScreen>
      <SendPaymentScreen contactId={contactId} />
    </LazyScreen>
  );
}

const dashpaySendPaymentRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/send-payment/$contactId",
  component: SendPaymentWrapper,
});

const dashpayQrGeneratorRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/qr-generator",
  component: () => (
    <LazyScreen>
      <QRCodeGeneratorScreen />
    </LazyScreen>
  ),
});

const dashpayQrScannerRoute = createRoute({
  getParentRoute: () => dashpayRoute,
  path: "/qr-scanner",
  component: () => (
    <LazyScreen>
      <QRScannerScreen />
    </LazyScreen>
  ),
});

const identitiesRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/identities",
  component: () => (
    <LazyScreen>
      <IdentitiesScreen />
    </LazyScreen>
  ),
});

const contractsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/contracts",
  component: () => <Outlet />,
});

const contractsIndexRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/",
  component: () => (
    <LazyScreen>
      <DocumentQueryScreen />
    </LazyScreen>
  ),
});

const contractsAddRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/add-contracts",
  component: () => (
    <LazyScreen>
      <AddContractsScreen />
    </LazyScreen>
  ),
});

const contractsRegisterRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/register",
  component: () => (
    <LazyScreen>
      <RegisterContractScreen />
    </LazyScreen>
  ),
});

const contractsUpdateRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/update-contract",
  component: () => (
    <LazyScreen>
      <UpdateContractScreen />
    </LazyScreen>
  ),
});

const contractsCreateDocumentRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/create-document",
  component: () => (
    <LazyScreen>
      <DocumentActionScreen actionType="create" />
    </LazyScreen>
  ),
});

const contractsDeleteDocumentRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/delete-document",
  component: () => (
    <LazyScreen>
      <DocumentActionScreen actionType="delete" />
    </LazyScreen>
  ),
});

const contractsReplaceDocumentRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/replace-document",
  component: () => (
    <LazyScreen>
      <DocumentActionScreen actionType="replace" />
    </LazyScreen>
  ),
});

const contractsTransferDocumentRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/transfer-document",
  component: () => (
    <LazyScreen>
      <DocumentActionScreen actionType="transfer" />
    </LazyScreen>
  ),
});

const contractsPurchaseDocumentRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/purchase-document",
  component: () => (
    <LazyScreen>
      <DocumentActionScreen actionType="purchase" />
    </LazyScreen>
  ),
});

const contractsSetDocumentPriceRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/set-document-price",
  component: () => (
    <LazyScreen>
      <DocumentActionScreen actionType="setPrice" />
    </LazyScreen>
  ),
});

const contractsGroupActionsRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/group-actions",
  component: () => (
    <LazyScreen>
      <GroupActionsScreen />
    </LazyScreen>
  ),
});

const contractsDpnsActiveRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-active",
  component: () => (
    <LazyScreen>
      <DpnsActiveContestsScreen />
    </LazyScreen>
  ),
});

const contractsDpnsPastRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-past",
  component: () => (
    <LazyScreen>
      <DpnsPastContestsScreen />
    </LazyScreen>
  ),
});

const contractsDpnsOwnedRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-owned",
  component: () => (
    <LazyScreen>
      <DpnsOwnedNamesScreen />
    </LazyScreen>
  ),
});

const contractsDpnsScheduledRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-scheduled",
  component: () => (
    <LazyScreen>
      <DpnsScheduledVotesScreen />
    </LazyScreen>
  ),
});

const contractsDpnsRegisterRoute = createRoute({
  getParentRoute: () => contractsRoute,
  path: "/dpns-register",
  component: () => (
    <LazyScreen>
      <DpnsRegisterNameScreen />
    </LazyScreen>
  ),
});

const tokensRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/tokens",
  component: () => <Outlet />,
});

const tokensIndexRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/",
  component: () => (
    <LazyScreen>
      <TokenMyTokensScreen />
    </LazyScreen>
  ),
});

const tokensSearchRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/search",
  component: () => (
    <LazyScreen>
      <TokenSearchScreen />
    </LazyScreen>
  ),
});

const tokensCreatorRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/creator",
  component: () => (
    <LazyScreen>
      <TokenCreatorScreen />
    </LazyScreen>
  ),
});

const tokensAddByIdRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/add-by-id",
  component: () => (
    <LazyScreen>
      <TokenAddByIdScreen />
    </LazyScreen>
  ),
});

const tokensTransferRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/transfer",
  component: () => (
    <LazyScreen>
      <TokenTransferScreen />
    </LazyScreen>
  ),
});

const tokensMintRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/mint",
  component: () => (
    <LazyScreen>
      <TokenMintScreen />
    </LazyScreen>
  ),
});

const tokensBurnRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/burn",
  component: () => (
    <LazyScreen>
      <TokenBurnScreen />
    </LazyScreen>
  ),
});

const tokensFreezeRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/freeze",
  component: () => (
    <LazyScreen>
      <TokenFreezeScreen />
    </LazyScreen>
  ),
});

const tokensUnfreezeRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/unfreeze",
  component: () => (
    <LazyScreen>
      <TokenUnfreezeScreen />
    </LazyScreen>
  ),
});

const tokensDestroyFrozenRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/destroy-frozen",
  component: () => (
    <LazyScreen>
      <TokenDestroyFrozenFundsScreen />
    </LazyScreen>
  ),
});

const tokensPauseRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/pause",
  component: () => (
    <LazyScreen>
      <TokenPauseScreen />
    </LazyScreen>
  ),
});

const tokensResumeRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/resume",
  component: () => (
    <LazyScreen>
      <TokenResumeScreen />
    </LazyScreen>
  ),
});

const tokensClaimRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/claim",
  component: () => (
    <LazyScreen>
      <TokenClaimScreen />
    </LazyScreen>
  ),
});

const tokensViewClaimsRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/view-claims",
  component: () => (
    <LazyScreen>
      <TokenViewClaimsScreen />
    </LazyScreen>
  ),
});

const tokensSetPriceRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/set-price",
  component: () => (
    <LazyScreen>
      <TokenSetPriceScreen />
    </LazyScreen>
  ),
});

const tokensPurchaseRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/purchase",
  component: () => (
    <LazyScreen>
      <TokenPurchaseScreen />
    </LazyScreen>
  ),
});

const tokensUpdateConfigRoute = createRoute({
  getParentRoute: () => tokensRoute,
  path: "/update-config",
  component: () => (
    <LazyScreen>
      <TokenUpdateConfigScreen />
    </LazyScreen>
  ),
});

const walletsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/wallets",
  component: () => <Outlet />,
});

const walletsIndexRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/",
  component: () => (
    <LazyScreen>
      <WalletsScreen />
    </LazyScreen>
  ),
});

const walletsCreateRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/create",
  component: () => (
    <LazyScreen>
      <CreateWalletScreen />
    </LazyScreen>
  ),
});

const walletsImportRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/import",
  component: () => (
    <LazyScreen>
      <ImportWalletScreen />
    </LazyScreen>
  ),
});

const walletsSendRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/send/$type",
  component: () => (
    <LazyScreen>
      <SendScreen />
    </LazyScreen>
  ),
});

const walletsSingleKeySendRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/send-single-key",
  component: () => (
    <LazyScreen>
      <SingleKeySendScreen />
    </LazyScreen>
  ),
});

const walletsCreateAssetLockRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/asset-locks/create",
  component: () => (
    <LazyScreen>
      <CreateAssetLockScreen />
    </LazyScreen>
  ),
});

const walletsAssetLockDetailRoute = createRoute({
  getParentRoute: () => walletsRoute,
  path: "/asset-locks/$txid",
  component: () => (
    <LazyScreen>
      <AssetLockDetailScreen />
    </LazyScreen>
  ),
});

const toolsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/tools",
  component: () => <Outlet />,
});

const toolsIndexRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/",
  component: () => (
    <LazyScreen>
      <ToolsScreen />
    </LazyScreen>
  ),
});

const toolsPlatformInfoRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/platform-info",
  component: () => (
    <LazyScreen>
      <PlatformInfoScreen />
    </LazyScreen>
  ),
});

const toolsProofLogRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/proof-log",
  component: () => (
    <LazyScreen>
      <ProofLogScreen />
    </LazyScreen>
  ),
});

const toolsTransitionRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/transition-visualizer",
  component: () => (
    <LazyScreen>
      <TransitionVisualizerScreen />
    </LazyScreen>
  ),
});

const toolsDocumentRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/document-visualizer",
  component: () => (
    <LazyScreen>
      <DocumentVisualizerScreen />
    </LazyScreen>
  ),
});

const toolsProofVisualizerRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/proof-visualizer",
  component: () => (
    <LazyScreen>
      <ProofVisualizerScreen />
    </LazyScreen>
  ),
});

const toolsMnListRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/masternode-list",
  component: () => (
    <LazyScreen>
      <MasternodeListDiffScreen />
    </LazyScreen>
  ),
});

const toolsContractRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/contract-visualizer",
  component: () => (
    <LazyScreen>
      <ContractVisualizerScreen />
    </LazyScreen>
  ),
});

const toolsGroveStarkRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/grovestark",
  component: () => (
    <LazyScreen>
      <GroveSTARKScreen />
    </LazyScreen>
  ),
});

const toolsAddressBalanceRoute = createRoute({
  getParentRoute: () => toolsRoute,
  path: "/address-balance",
  component: () => (
    <LazyScreen>
      <AddressBalanceScreen />
    </LazyScreen>
  ),
});

const settingsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings",
  component: () => (
    <LazyScreen>
      <NetworkChooserScreen />
    </LazyScreen>
  ),
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
      dashpayAddContactRoute,
      dashpayContactDetailsRoute,
      dashpayContactProfileRoute,
      dashpayContactInfoEditorRoute,
      dashpaySendPaymentRoute,
      dashpayQrGeneratorRoute,
      dashpayQrScannerRoute,
    ]),
    identitiesRoute,
    contractsRoute.addChildren([
      contractsIndexRoute,
      contractsAddRoute,
      contractsRegisterRoute,
      contractsUpdateRoute,
      contractsCreateDocumentRoute,
      contractsDeleteDocumentRoute,
      contractsReplaceDocumentRoute,
      contractsTransferDocumentRoute,
      contractsPurchaseDocumentRoute,
      contractsSetDocumentPriceRoute,
      contractsGroupActionsRoute,
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
      tokensAddByIdRoute,
      tokensTransferRoute,
      tokensMintRoute,
      tokensBurnRoute,
      tokensFreezeRoute,
      tokensUnfreezeRoute,
      tokensDestroyFrozenRoute,
      tokensPauseRoute,
      tokensResumeRoute,
      tokensClaimRoute,
      tokensViewClaimsRoute,
      tokensSetPriceRoute,
      tokensPurchaseRoute,
      tokensUpdateConfigRoute,
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
