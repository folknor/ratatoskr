import type React from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "@/components/ui/Modal";

interface SetupProviderCredentialsProps {
  provider: "google" | "microsoft";
  onSubmit: (clientId: string, clientSecret: string | null) => void;
  onCancel: () => void;
  initialClientId?: string;
  initialClientSecret?: string | null;
  title?: string;
  submitLabel?: string;
  error?: string | null;
}

export function SetupProviderCredentials({
  provider,
  onSubmit,
  onCancel,
  initialClientId = "",
  initialClientSecret = "",
  title,
  submitLabel,
  error,
}: SetupProviderCredentialsProps): React.ReactNode {
  const { t } = useTranslation("accounts");
  const [clientId, setClientId] = useState(initialClientId);
  const [clientSecret, setClientSecret] = useState(initialClientSecret ?? "");

  const handleSubmit = (): void => {
    const trimmedId = clientId.trim();
    if (!trimmedId) return;
    const trimmedSecret = clientSecret.trim();
    if (provider === "google" && !trimmedSecret) return;
    onSubmit(trimmedId, provider === "google" ? trimmedSecret : null);
  };

  const isValid =
    provider === "microsoft"
      ? clientId.trim().length > 0
      : clientId.trim().length > 0 && clientSecret.trim().length > 0;

  const resolvedTitle =
    title ??
    (provider === "google"
      ? t("googleApiSetup", "Google API Setup")
      : t("microsoftApiSetup", "Microsoft API Setup"));

  return (
    <Modal
      isOpen={true}
      onClose={onCancel}
      title={resolvedTitle}
      width="w-full max-w-lg"
    >
      <div className="p-4">
        {provider === "google" ? (
          <>
            <p className="text-text-secondary text-sm mb-4">
              {t(
                "googleSetupDescription",
                "To connect Gmail accounts, you need a Google Cloud OAuth Client ID.",
              )}
            </p>
            <ol className="text-text-secondary text-sm mb-4 space-y-1 list-decimal list-inside">
              <li>
                {t("googleStep1", "Go to the")}{" "}
                <span className="text-accent">Google Cloud Console</span>
              </li>
              <li>
                {t("googleStep2", "Create a project (or use an existing one)")}
              </li>
              <li>{t("googleStep3", "Enable the Gmail API")}</li>
              <li>
                {t(
                  "googleStep4",
                  "Create OAuth 2.0 credentials (Web application type)",
                )}
              </li>
              <li>
                {t("googleStep5", "Add")}{" "}
                <code className="bg-bg-tertiary px-1 rounded text-xs">
                  http://127.0.0.1:17248
                </code>{" "}
                {t("googleStep5b", "as an authorized redirect URI")}
              </li>
              <li>
                {t("googleStep6", "Copy the Client ID and Client Secret below")}
              </li>
            </ol>
          </>
        ) : (
          <>
            <p className="text-text-secondary text-sm mb-4">
              {t(
                "microsoftSetupDescription",
                "To connect Microsoft accounts, you need an Azure App Registration Client ID.",
              )}
            </p>
            <ol className="text-text-secondary text-sm mb-4 space-y-1 list-decimal list-inside">
              <li>
                {t("microsoftStep1", "Go to the")}{" "}
                <span className="text-accent">Azure Portal</span>{" "}
                {t("microsoftStep1b", "(App Registrations)")}
              </li>
              <li>{t("microsoftStep2", "Register a new application")}</li>
              <li>
                {t("microsoftStep3", "Add redirect URI")}{" "}
                <code className="bg-bg-tertiary px-1 rounded text-xs">
                  http://localhost:17248
                </code>
              </li>
              <li>
                {t(
                  "microsoftStep4",
                  'Under Authentication, enable "Allow public client flows"',
                )}
              </li>
              <li>
                {t("microsoftStep5", "Copy the Application (client) ID below")}
              </li>
            </ol>
          </>
        )}

        {Boolean(error) && (
          <div className="mb-4 rounded-lg border border-danger/20 bg-danger/10 p-3 text-sm text-danger">
            {error}
          </div>
        )}

        <input
          type="text"
          value={clientId}
          onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
            setClientId(e.target.value)
          }
          placeholder={
            provider === "google"
              ? t("googleClientIdPlaceholder", "Paste your Client ID here...")
              : t(
                  "microsoftClientIdPlaceholder",
                  "Azure App Registration Client ID",
                )
          }
          className="w-full px-3 py-2 bg-bg-secondary border border-border-primary rounded-lg text-sm mb-3 outline-none focus:border-accent"
        />

        {provider === "google" && (
          <>
            <input
              type="password"
              value={clientSecret}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                setClientSecret(e.target.value)
              }
              placeholder={t(
                "googleClientSecretPlaceholder",
                "Paste your Client Secret here...",
              )}
              className="w-full px-3 py-2 bg-bg-secondary border border-border-primary rounded-lg text-sm mb-1 outline-none focus:border-accent"
            />
            <p className="text-text-tertiary text-xs mb-4">
              {t(
                "clientSecretHint",
                "Required for Web application credentials",
              )}
            </p>
          </>
        )}

        <div className="flex gap-3 justify-end">
          <button
            type="button"
            onClick={onCancel}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            {t("common:cancel", "Cancel")}
          </button>
          <button
            type="button"
            onClick={handleSubmit}
            disabled={!isValid}
            className="px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {submitLabel ?? t("continueSetup", "Continue")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
