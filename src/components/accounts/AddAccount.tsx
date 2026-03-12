import { invoke } from "@tauri-apps/api/core";
import { Calendar, Mail } from "lucide-react";
import type React from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "@/components/ui/Modal";
import { useAccountStore } from "@/stores/accountStore";
import { AddCalDavAccount } from "./AddCalDavAccount";
import { AddGraphAccount } from "./AddGraphAccount";
import { AddImapAccount } from "./AddImapAccount";
import { SetupProviderCredentials } from "./SetupProviderCredentials";

interface AddAccountProps {
  onClose: () => void;
  onSuccess: () => void;
}

interface GmailAccountResult {
  id: string;
  email: string;
  displayName: string;
  avatarUrl: string | null;
  isActive: boolean;
  provider: string;
}

type View =
  | "select-provider"
  | "gmail-credentials"
  | "gmail-auth"
  | "imap"
  | "caldav"
  | "graph";

export function AddAccount({
  onClose,
  onSuccess,
}: AddAccountProps): React.ReactNode {
  const { t } = useTranslation("accounts");
  const [view, setView] = useState<View>("select-provider");
  const [status, setStatus] = useState<
    "idle" | "checking" | "authenticating" | "error"
  >("idle");
  const [error, setError] = useState<string | null>(null);
  const addAccount = useAccountStore((s) => s.addAccount);

  const handleGmailCredentials = (
    clientId: string,
    clientSecret: string | null,
  ): void => {
    setView("gmail-auth");
    void handleAddGmailAccount(clientId, clientSecret);
  };

  const handleAddGmailAccount = async (
    clientId: string,
    clientSecret: string | null,
  ): Promise<void> => {
    setStatus("authenticating");
    setError(null);

    try {
      const account = await invoke<GmailAccountResult>(
        "account_create_gmail_via_oauth",
        { clientId, clientSecret },
      );

      addAccount({
        id: account.id,
        email: account.email,
        displayName: account.displayName,
        avatarUrl: account.avatarUrl,
        isActive: account.isActive,
        provider: account.provider,
      });

      onSuccess();
    } catch (err) {
      console.error("Add account error:", err);
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      setStatus("error");
      setView("gmail-credentials");
    }
  };

  if (view === "gmail-credentials") {
    return (
      <SetupProviderCredentials
        provider="google"
        onSubmit={handleGmailCredentials}
        onCancel={onClose}
      />
    );
  }

  if (view === "caldav") {
    return (
      <AddCalDavAccount
        onClose={onClose}
        onSuccess={onSuccess}
        onBack={(): void => setView("select-provider")}
      />
    );
  }

  if (view === "imap") {
    return (
      <AddImapAccount
        onClose={onClose}
        onSuccess={onSuccess}
        onBack={(): void => setView("select-provider")}
      />
    );
  }

  if (view === "graph") {
    return (
      <AddGraphAccount
        onClose={onClose}
        onSuccess={onSuccess}
        onBack={(): void => setView("select-provider")}
      />
    );
  }

  if (view === "gmail-auth") {
    return (
      <Modal
        isOpen={true}
        onClose={onClose}
        title={t("addGmailAccount")}
        width="w-full max-w-md"
      >
        <div className="p-4">
          <p className="text-text-secondary text-sm mb-6">
            {t("gmailSignInDescription")}
          </p>

          {error != null && (
            <div className="bg-danger/10 border border-danger/20 rounded-lg p-3 mb-4 text-sm text-danger">
              {error}
            </div>
          )}

          {status === "authenticating" && (
            <div className="text-center py-4 text-text-secondary text-sm">
              <div className="mb-2">{t("waitingForSignIn")}</div>
              <div className="text-xs text-text-tertiary">
                {t("completeSignIn")}
              </div>
            </div>
          )}

          <div className="flex gap-3 justify-between">
            <button
              type="button"
              onClick={(): void => {
                setView("select-provider");
                setStatus("idle");
                setError(null);
              }}
              className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
            >
              {t("common:back")}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
            >
              {t("common:cancel")}
            </button>
          </div>
        </div>
      </Modal>
    );
  }

  // Provider selection view
  return (
    <Modal
      isOpen={true}
      onClose={onClose}
      title={t("addAccount")}
      width="w-full max-w-md"
    >
      <div className="p-4">
        <p className="text-text-secondary text-sm mb-4">
          {t("chooseConnectionMethod")}
        </p>

        <div className="space-y-3">
          <button
            type="button"
            onClick={(): void => setView("gmail-credentials")}
            className="w-full flex items-center gap-4 p-4 rounded-lg border border-border-primary bg-bg-secondary hover:bg-bg-hover transition-colors text-left group"
          >
            <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-bg-tertiary flex items-center justify-center">
              <svg
                className="w-5 h-5"
                viewBox="0 0 24 24"
                aria-label="Google logo"
              >
                <path
                  d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z"
                  fill="#4285F4"
                />
                <path
                  d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
                  fill="#34A853"
                />
                <path
                  d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
                  fill="#FBBC05"
                />
                <path
                  d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
                  fill="#EA4335"
                />
              </svg>
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-text-primary group-hover:text-accent transition-colors">
                {t("googleGmail")}
              </div>
              <div className="text-xs text-text-tertiary mt-0.5">
                {t("gmailOauthDescription")}
              </div>
            </div>
          </button>

          <button
            type="button"
            onClick={(): void => setView("graph")}
            className="w-full flex items-center gap-4 p-4 rounded-lg border border-border-primary bg-bg-secondary hover:bg-bg-hover transition-colors text-left group"
          >
            <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-bg-tertiary flex items-center justify-center">
              <svg
                className="w-5 h-5"
                viewBox="0 0 21 21"
                aria-label="Microsoft logo"
              >
                <rect x="1" y="1" width="9" height="9" fill="#f25022" />
                <rect x="11" y="1" width="9" height="9" fill="#7fba00" />
                <rect x="1" y="11" width="9" height="9" fill="#00a4ef" />
                <rect x="11" y="11" width="9" height="9" fill="#ffb900" />
              </svg>
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-text-primary group-hover:text-accent transition-colors">
                {t("microsoftOutlook", "Microsoft Outlook")}
              </div>
              <div className="text-xs text-text-tertiary mt-0.5">
                {t(
                  "microsoftDescription",
                  "Outlook.com, Hotmail, Microsoft 365",
                )}
              </div>
            </div>
          </button>

          <button
            type="button"
            onClick={(): void => setView("imap")}
            className="w-full flex items-center gap-4 p-4 rounded-lg border border-border-primary bg-bg-secondary hover:bg-bg-hover transition-colors text-left group"
          >
            <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-bg-tertiary flex items-center justify-center">
              <Mail className="w-5 h-5 text-text-secondary" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-text-primary group-hover:text-accent transition-colors">
                {t("imapSmtp")}
              </div>
              <div className="text-xs text-text-tertiary mt-0.5">
                {t("imapDescription")}
              </div>
            </div>
          </button>

          <button
            type="button"
            onClick={(): void => setView("caldav")}
            className="w-full flex items-center gap-4 p-4 rounded-lg border border-border-primary bg-bg-secondary hover:bg-bg-hover transition-colors text-left group"
          >
            <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-bg-tertiary flex items-center justify-center">
              <Calendar className="w-5 h-5 text-text-secondary" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-text-primary group-hover:text-accent transition-colors">
                {t("caldavCalendar")}
              </div>
              <div className="text-xs text-text-tertiary mt-0.5">
                {t("caldavDescription")}
              </div>
            </div>
          </button>
        </div>

        <div className="flex justify-end mt-4">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            {t("common:cancel")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
