import type React from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "@/components/ui/Modal";
import { useAccountStore } from "@/stores/accountStore";
import { getCurrentUnixTimestamp } from "@/utils/timestamp";
import {
  getOAuthProvider,
  insertGraphAccount,
  startProviderOAuthFlow,
} from "@/core/accounts";
import { getSetting } from "@/services/db/settings";

interface AddGraphAccountProps {
  onClose: () => void;
  onSuccess: () => void;
  onBack: () => void;
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
      // Get Microsoft client ID from settings
      const clientId = await getSetting("microsoft_client_id");
      if (!clientId) {
        setError(
          "Microsoft Client ID not configured. Go to Settings to set it up.",
        );
        setStatus("error");
        return;
      }

      const provider = getOAuthProvider("microsoft_graph");
      if (!provider) {
        setError("Microsoft Graph OAuth provider not found");
        setStatus("error");
        return;
      }

      setStatus("authenticating");

      const { tokens, userInfo } = await startProviderOAuthFlow(
        provider,
        clientId,
      );

      if (!userInfo.email) {
        setError("Could not determine email address from Microsoft account");
        setStatus("error");
        return;
      }

      setStatus("testing");

      const accountId = crypto.randomUUID();
      const expiresAt = getCurrentUnixTimestamp() + tokens.expires_in;

      // Save account to DB
      await insertGraphAccount({
        id: accountId,
        email: userInfo.email,
        displayName: userInfo.name || null,
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token ?? "",
        tokenExpiresAt: expiresAt,
      });

      // Initialize Rust Graph client
      await invoke("graph_init_client", { accountId });

      // Test connection
      const testResult = await invoke<{ success: boolean; message: string }>(
        "graph_test_connection",
        { accountId },
      );

      if (!testResult.success) {
        // Clean up on failure
        await invoke("graph_remove_client", { accountId }).catch(() => {});
        // Delete the account from DB
        const { deleteAccount } = await import("@/core/accounts");
        await deleteAccount(accountId);
        setError(`Connection test failed: ${testResult.message}`);
        setStatus("error");
        return;
      }

      // Add to store
      addAccount({
        id: accountId,
        email: userInfo.email,
        displayName: userInfo.name || null,
        avatarUrl: null,
        isActive: true,
        provider: "graph",
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
