import { invoke } from "@tauri-apps/api/core";
import type React from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "@/components/ui/Modal";
import { useAccountStore } from "@/stores/accountStore";
import { SetupProviderCredentials } from "./SetupProviderCredentials";

interface AddGraphAccountProps {
  onClose: () => void;
  onSuccess: () => void;
  onBack: () => void;
}

interface GraphAccountResult {
  id: string;
  email: string;
  displayName: string;
  avatarUrl: string | null;
  isActive: boolean;
  provider: string;
}

type GraphView = "credentials" | "auth";

export function AddGraphAccount({
  onClose,
  onSuccess,
  onBack,
}: AddGraphAccountProps): React.ReactNode {
  const { t } = useTranslation("accounts");
  const [graphView, setGraphView] = useState<GraphView>("credentials");
  const [status, setStatus] = useState<
    "idle" | "checking" | "authenticating" | "error"
  >("idle");
  const [error, setError] = useState<string | null>(null);
  const addAccount = useAccountStore((s) => s.addAccount);

  const handleCredentials = (clientId: string): void => {
    setGraphView("auth");
    void handleSignIn(clientId);
  };

  const handleSignIn = async (clientId: string): Promise<void> => {
    setStatus("authenticating");
    setError(null);

    try {
      const account = await invoke<GraphAccountResult>(
        "account_create_graph_via_oauth",
        { clientId },
      );

      addAccount({
        id: account.id,
        email: account.email,
        displayName: account.displayName || null,
        avatarUrl: account.avatarUrl || null,
        isActive: account.isActive,
        provider: account.provider,
      });

      onSuccess();
    } catch (err) {
      console.error("Add Graph account error:", err);
      setError(err instanceof Error ? err.message : String(err));
      setStatus("error");
      setGraphView("credentials");
    }
  };

  if (graphView === "credentials") {
    return (
      <SetupProviderCredentials
        provider="microsoft"
        onSubmit={(clientId: string): void => handleCredentials(clientId)}
        onCancel={onBack}
      />
    );
  }

  return (
    <Modal
      isOpen={true}
      onClose={onClose}
      title={t("addMicrosoftAccount", "Add Microsoft Account")}
      width="w-full max-w-md"
    >
      <div className="p-4">
        <p className="text-text-secondary text-sm mb-6">
          {t(
            "microsoftSignInDescription",
            "Sign in with your Microsoft account to connect Outlook, Hotmail, or Microsoft 365 email.",
          )}
        </p>

        {error != null && (
          <div className="bg-danger/10 border border-danger/20 rounded-lg p-3 mb-4 text-sm text-danger">
            {error}
          </div>
        )}

        {status === "authenticating" && (
          <div className="text-center py-4 text-text-secondary text-sm">
            <div className="mb-2">
              {t("waitingForSignIn", "Waiting for sign-in...")}
            </div>
            <div className="text-xs text-text-tertiary">
              {t(
                "completeSignIn",
                "Complete the sign-in process in your browser",
              )}
            </div>
          </div>
        )}
        <div className="flex gap-3 justify-between">
          <button
            type="button"
            onClick={onBack}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            {t("common:back", "Back")}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            {t("common:cancel", "Cancel")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
