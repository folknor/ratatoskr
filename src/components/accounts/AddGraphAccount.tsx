import { invoke } from "@tauri-apps/api/core";
import type React from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "@/components/ui/Modal";
import { useAccountStore } from "@/stores/accountStore";

interface AddGraphAccountProps {
  onClose: () => void;
  onSuccess: () => void;
  onBack: () => void;
}

interface GraphAccountResult {
  id: string;
  email: string;
  displayName: string;
  avatarUrl: string;
  isActive: boolean;
  provider: string;
}

export function AddGraphAccount({
  onClose,
  onSuccess,
  onBack,
}: AddGraphAccountProps): React.ReactNode {
  const { t } = useTranslation("accounts");
  const [status, setStatus] = useState<
    "idle" | "checking" | "authenticating" | "testing" | "error"
  >("idle");
  const [error, setError] = useState<string | null>(null);
  const addAccount = useAccountStore((s) => s.addAccount);

  const handleSignIn = async (): Promise<void> => {
    setStatus("checking");
    setError(null);

    try {
      setStatus("authenticating");

      setStatus("testing");

      const account = await invoke<GraphAccountResult>(
        "account_create_graph_via_oauth",
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
    }
  };

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

        {status === "testing" && (
          <div className="text-center py-4 text-text-secondary text-sm">
            {t("testingConnection", "Testing connection...")}
          </div>
        )}

        <div className="flex gap-3 justify-between">
          <button
            type="button"
            onClick={(): void => {
              onBack();
            }}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            {t("common:back", "Back")}
          </button>
          <div className="flex gap-3">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
            >
              {t("common:cancel", "Cancel")}
            </button>
            <button
              type="button"
              onClick={handleSignIn}
              disabled={
                status === "authenticating" ||
                status === "checking" ||
                status === "testing"
              }
              className="px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {status === "authenticating"
                ? t("common:waiting", "Waiting...")
                : status === "checking" || status === "testing"
                  ? t("common:checking", "Checking...")
                  : t("signInWithMicrosoft", "Sign in with Microsoft")}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
