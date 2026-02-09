import { useCallback } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Wallet, Download, Compass } from "lucide-react";
import { commands } from "@/bindings";
import dashLogo from "@/assets/dashlogo.svg";

type OnboardingAction = "create-wallet" | "import-wallet" | "just-browse";

interface ActionCardProps {
  icon: React.ElementType;
  title: string;
  description: string;
  onClick: () => void;
}

function ActionCard({ icon: Icon, title, description, onClick }: ActionCardProps) {
  return (
    <button
      onClick={onClick}
      className="group flex w-56 flex-col items-center gap-3 rounded-lg border border-border bg-background p-6 shadow-sm transition-all hover:border-primary/40 hover:shadow-md hover:shadow-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 transition-colors group-hover:bg-primary/20">
        <Icon className="h-6 w-6 text-primary" />
      </div>
      <span className="text-sm font-semibold text-foreground">{title}</span>
      <span className="text-center text-xs text-muted-foreground">{description}</span>
    </button>
  );
}

export function WelcomeScreen() {
  const navigate = useNavigate();

  const handleAction = useCallback(
    async (action: OnboardingAction) => {
      // Mark onboarding as completed in backend
      try {
        await commands.settingsUpdateOnboardingCompleted(true);
      } catch {
        // Backend may not be available in browser-only mode
      }

      // Navigate based on selection
      switch (action) {
        case "create-wallet":
          navigate({ to: "/wallets" });
          break;
        case "import-wallet":
          navigate({ to: "/wallets" });
          break;
        case "just-browse":
          navigate({ to: "/identities" });
          break;
      }
    },
    [navigate],
  );

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-background">
      <div className="flex flex-col items-center gap-6">
        {/* Logo */}
        <img src={dashLogo} alt="Dash" className="h-14 w-auto" />

        {/* Title */}
        <h1 className="text-3xl font-bold text-foreground">Welcome to Dash Evo Tool</h1>

        {/* Subtitle */}
        <p className="text-base text-muted-foreground">Your gateway to decentralized data</p>

        {/* Spacer */}
        <div className="h-4" />

        {/* Instructional text */}
        <p className="text-sm text-muted-foreground">Select an option to get started:</p>

        {/* Action cards */}
        <div className="flex gap-4">
          <ActionCard
            icon={Wallet}
            title="Create Wallet"
            description="Start fresh with a new HD wallet"
            onClick={() => handleAction("create-wallet")}
          />
          <ActionCard
            icon={Download}
            title="Import Wallet"
            description="Load a wallet you already have"
            onClick={() => handleAction("import-wallet")}
          />
          <ActionCard
            icon={Compass}
            title="Just Explore"
            description="Explore without setting up"
            onClick={() => handleAction("just-browse")}
          />
        </div>
      </div>
    </div>
  );
}
