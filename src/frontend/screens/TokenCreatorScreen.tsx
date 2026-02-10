import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, Coins } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Island, PageHeader } from "@/components/layout";
import { TokenCreatorWizard } from "@/components/token/TokenCreatorWizard";

/**
 * TokenCreatorScreen — multi-step wizard for creating new token contracts.
 *
 * The screen wraps TokenCreatorWizard (7 steps) and provides:
 * - Page header with back navigation to /tokens
 * - Island container for the wizard content
 * - Cancel handler that navigates back
 */
export function TokenCreatorScreen() {
  const navigate = useNavigate();

  return (
    <div className="flex flex-col gap-4 h-full">
      <PageHeader title="Token Creator" icon={Coins}>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => navigate({ to: "/tokens" })}
          data-testid="back-to-tokens"
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back to Tokens
        </Button>
      </PageHeader>

      <Island className="flex-1 min-h-0 p-6">
        <TokenCreatorWizard
          onCancel={() => navigate({ to: "/tokens" })}
        />
      </Island>
    </div>
  );
}
