import { CheckCircle2, KeyRound, Loader2, ShieldCheck } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import type { AuthMode, FormState } from "./addImapAccountTypes";
import { inputClass, labelClass } from "./addImapAccountTypes";

export interface AddImapAccountBasicStepProps {
  form: FormState;
  updateForm: <K extends keyof FormState>(key: K, value: FormState[K]) => void;
  handleEmailBlur: () => void;
  isOAuth: boolean;
  hasOAuthTokens: boolean;
  detectedAuthMethods: AuthMode[];
  detectedOAuthProviderId: string | null;
  detectedProviderName: string | null;
  detectedHelpUrl: string | null;
  oauthConnecting: boolean;
  oauthError: string | null;
  onOAuthConnect: (providerId: string) => void;
}

export function AddImapAccountBasicStep({
  form,
  updateForm,
  handleEmailBlur,
  isOAuth,
  hasOAuthTokens,
  detectedAuthMethods,
  detectedOAuthProviderId,
  detectedProviderName,
  detectedHelpUrl,
  oauthConnecting,
  oauthError,
  onOAuthConnect,
}: AddImapAccountBasicStepProps): React.ReactNode {
  const { t } = useTranslation("accounts");

  const renderAuthModeSelector = (): React.ReactNode => {
    const showOAuth =
      detectedAuthMethods.includes("oauth2") || form.authMode === "oauth2";
    if (!showOAuth) return null;

    return (
      <div className="mb-4">
        {/* biome-ignore lint/a11y/noLabelWithoutControl: label describes a button group, not a single input */}
        <label className={labelClass}>{t("authMethod")}</label>
        <div className="flex gap-2">
          {detectedAuthMethods.includes("password") && (
            <button
              type="button"
              onClick={() => updateForm("authMode", "password")}
              className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-sm rounded-lg border transition-colors ${
                form.authMode === "password"
                  ? "border-accent bg-accent/10 text-accent"
                  : "border-border-primary bg-bg-secondary text-text-secondary hover:bg-bg-hover"
              }`}
            >
              <KeyRound className="w-4 h-4" />
              {t("password")}
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              updateForm("authMode", "oauth2");
              if (detectedOAuthProviderId) {
                updateForm("oauthProvider", detectedOAuthProviderId);
              }
            }}
            className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-sm rounded-lg border transition-colors ${
              form.authMode === "oauth2"
                ? "border-accent bg-accent/10 text-accent"
                : "border-border-primary bg-bg-secondary text-text-secondary hover:bg-bg-hover"
            }`}
          >
            <ShieldCheck className="w-4 h-4" />
            {t("oauth2")}
          </button>
        </div>
      </div>
    );
  };

  const renderOAuthSection = (): React.ReactNode => {
    const providerId = form.oauthProvider ?? detectedOAuthProviderId;
    const providerName =
      providerId === "microsoft"
        ? "Microsoft"
        : providerId === "yahoo"
          ? "Yahoo"
          : "Provider";

    return (
      <div className="space-y-3">
        <div>
          <label htmlFor="oauth-client-id" className={labelClass}>
            {t("clientId")}
          </label>
          <input
            id="oauth-client-id"
            type="text"
            value={form.oauthClientId}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              updateForm("oauthClientId", e.target.value)
            }
            placeholder={`${providerName} app Client ID`}
            className={inputClass}
            disabled={hasOAuthTokens}
          />
        </div>
        <div>
          <label htmlFor="oauth-client-secret" className={labelClass}>
            {t("clientSecretOptional")}
          </label>
          <input
            id="oauth-client-secret"
            type="password"
            value={form.oauthClientSecret}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              updateForm("oauthClientSecret", e.target.value)
            }
            placeholder={t("leaveBlankPublic")}
            className={inputClass}
            disabled={hasOAuthTokens}
          />
        </div>

        {hasOAuthTokens ? (
          <div className="flex items-center gap-2 p-3 rounded-lg bg-success/10 border border-success/20">
            <CheckCircle2 className="w-4 h-4 text-success flex-shrink-0" />
            <div className="text-sm text-success">
              {t("connectedAs", { email: form.oauthEmail })}
            </div>
          </div>
        ) : (
          <button
            type="button"
            onClick={(): void => {
              if (providerId) void onOAuthConnect(providerId);
            }}
            disabled={oauthConnecting || !form.oauthClientId.trim()}
            className="w-full flex items-center justify-center gap-2 px-4 py-2.5 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {oauthConnecting ? (
              <>
                <Loader2 className="w-4 h-4 animate-spin" />
                {t("connecting")}
              </>
            ) : (
              <>
                <ShieldCheck className="w-4 h-4" />
                {t("signInWith", { provider: providerName })}
              </>
            )}
          </button>
        )}

        {oauthError != null && (
          <div className="bg-danger/10 border border-danger/20 rounded-lg p-3 text-sm text-danger">
            {oauthError}
          </div>
        )}

        <p className="text-xs text-text-tertiary">
          {t("registerAppWith", { provider: providerName })}{" "}
          {providerId === "microsoft" && (
            <>
              {t("registerAzure")}{" "}
              <code className="text-accent">{t("oauthRedirectUri")}</code>.
            </>
          )}
          {providerId === "yahoo" && (
            <>
              {t("registerYahoo")}{" "}
              <code className="text-accent">{t("oauthRedirectUri")}</code>.
            </>
          )}
        </p>
      </div>
    );
  };

  return (
    <div className="space-y-4">
      <div>
        <label htmlFor="imap-email" className={labelClass}>
          {t("emailAddress")}
        </label>
        <input
          id="imap-email"
          type="email"
          value={form.email}
          onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
            updateForm("email", e.target.value)
          }
          onBlur={handleEmailBlur}
          placeholder={t("emailPlaceholder")}
          className={inputClass}
          disabled={isOAuth && hasOAuthTokens}
        />
      </div>

      {renderAuthModeSelector()}

      {isOAuth ? (
        renderOAuthSection()
      ) : (
        <>
          <div>
            <label htmlFor="imap-display-name" className={labelClass}>
              {t("displayNameOptional")}
            </label>
            <input
              id="imap-display-name"
              type="text"
              value={form.displayName}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                updateForm("displayName", e.target.value)
              }
              placeholder={t("yourName")}
              className={inputClass}
            />
          </div>
          <div>
            <label htmlFor="imap-username" className={labelClass}>
              {t("usernameOptional")}
            </label>
            <input
              id="imap-username"
              type="text"
              value={form.imapUsername}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                updateForm("imapUsername", e.target.value)
              }
              placeholder={t("usernameHelp")}
              className={inputClass}
            />
            <p className="text-xs text-text-tertiary mt-1">
              {t("usernameDiffersHelp")}
            </p>
          </div>
          <div>
            <label htmlFor="imap-password" className={labelClass}>
              {t("password")}
            </label>
            <input
              id="imap-password"
              type="password"
              value={form.password}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                updateForm("password", e.target.value)
              }
              placeholder={t("enterPassword")}
              className={inputClass}
            />
            <p className="text-xs text-text-tertiary mt-1">
              {t("appPasswordHelp")}
              {detectedHelpUrl ? (
                <>
                  {" "}
                  <a
                    href={detectedHelpUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="text-accent hover:underline"
                  >
                    {t("appPasswordHelpLink", {
                      provider: detectedProviderName ?? t("provider"),
                    })}
                  </a>
                </>
              ) : null}
            </p>
          </div>
        </>
      )}

      {isOAuth === true && hasOAuthTokens === true && (
        <div>
          <label htmlFor="imap-display-name" className={labelClass}>
            {t("displayNameOptional")}
          </label>
          <input
            id="imap-display-name"
            type="text"
            value={form.displayName}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              updateForm("displayName", e.target.value)
            }
            placeholder={t("yourName")}
            className={inputClass}
          />
        </div>
      )}
    </div>
  );
}
