import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  ArrowRight,
  Mail,
  Send,
  Server,
  ShieldCheck,
} from "lucide-react";
import type React from "react";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "@/components/ui/Modal";
import {
  discoverSettings,
  getDefaultImapPort,
  getDefaultSmtpPort,
  getOAuthProvider,
  insertImapAccount,
  type SecurityType,
} from "@/core/accounts";
import { useAccountStore } from "@/stores/accountStore";
import { AddImapAccountBasicStep } from "./AddImapAccountBasicStep";
import { AddImapAccountImapStep } from "./AddImapAccountImapStep";
import { AddImapAccountSmtpStep } from "./AddImapAccountSmtpStep";
import { AddImapAccountTestStep } from "./AddImapAccountTestStep";
import type {
  AuthMode,
  FormState,
  Step,
  TestStatus,
} from "./addImapAccountTypes";
import { initialFormState } from "./addImapAccountTypes";
import { type StepDef, StepIndicator } from "./StepIndicator";

interface AddImapAccountProps {
  onClose: () => void;
  onSuccess: () => void;
  onBack: () => void;
}

interface OAuthAuthorizationResult {
  accessToken: string;
  refreshToken?: string | null;
  expiresIn: number;
  email: string;
  name: string;
  picture?: string | null;
}

const steps: Step[] = ["basic", "imap", "smtp", "test"];

const stepLabelKeys: Record<Step, string> = {
  basic: "stepAccount",
  imap: "stepIncoming",
  smtp: "stepOutgoing",
  test: "stepVerify",
};

const stepIcons: Record<Step, React.ReactNode> = {
  basic: <Mail className="w-4 h-4" />,
  imap: <Server className="w-4 h-4" />,
  smtp: <Send className="w-4 h-4" />,
  test: <ShieldCheck className="w-4 h-4" />,
};

/** Map UI security value ("ssl") to Rust config value ("tls") */
function mapSecurity(security: string): string {
  if (security === "ssl") return "tls";
  return security;
}

export function AddImapAccount({
  onClose,
  onSuccess,
  onBack,
}: AddImapAccountProps): React.ReactNode {
  const { t } = useTranslation("accounts");
  const [currentStep, setCurrentStep] = useState<Step>("basic");
  const [form, setForm] = useState<FormState>(initialFormState);
  const [imapTest, setImapTest] = useState<TestStatus>({ state: "idle" });
  const [smtpTest, setSmtpTest] = useState<TestStatus>({ state: "idle" });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [discoveryApplied, setDiscoveryApplied] = useState(false);
  const [oauthConnecting, setOauthConnecting] = useState(false);
  const [oauthError, setOauthError] = useState<string | null>(null);
  const [detectedAuthMethods, setDetectedAuthMethods] = useState<AuthMode[]>([
    "password",
  ]);
  const [detectedOAuthProviderId, setDetectedOAuthProviderId] = useState<
    string | null
  >(null);

  const addAccount = useAccountStore((s) => s.addAccount);

  const currentStepIndex = steps.indexOf(currentStep);

  const stepDefs: StepDef[] = steps.map((s) => ({
    key: s,
    label: t(stepLabelKeys[s]),
    icon: stepIcons[s],
  }));

  const updateForm = useCallback(
    <K extends keyof FormState>(key: K, value: FormState[K]) => {
      setForm((prev) => ({ ...prev, [key]: value }));
    },
    [],
  );

  const handleEmailBlur = useCallback((): void => {
    if (discoveryApplied) return;
    void discoverSettings(form.email).then((result) => {
      if (result && !form.imapHost && !form.smtpHost) {
        setForm((prev) => ({
          ...prev,
          imapHost: result.settings.imapHost,
          imapPort: result.settings.imapPort,
          imapSecurity: result.settings.imapSecurity,
          smtpHost: result.settings.smtpHost,
          smtpPort: result.settings.smtpPort,
          smtpSecurity: result.settings.smtpSecurity,
          acceptInvalidCerts: result.acceptInvalidCerts ?? false,
          // Auto-select OAuth2 if it's the only option (e.g. Outlook)
          authMode:
            result.authMethods[0] === "oauth2" ? "oauth2" : prev.authMode,
          oauthProvider: result.oauthProviderId ?? null,
        }));
        setDetectedAuthMethods(result.authMethods);
        setDetectedOAuthProviderId(result.oauthProviderId ?? null);
        setDiscoveryApplied(true);
      }
    });
  }, [form.email, form.imapHost, form.smtpHost, discoveryApplied]);

  const handleImapSecurityChange = useCallback(
    (security: SecurityType): void => {
      setForm((prev) => ({
        ...prev,
        imapSecurity: security,
        imapPort: getDefaultImapPort(security),
      }));
    },
    [],
  );

  const handleSmtpSecurityChange = useCallback(
    (security: SecurityType): void => {
      setForm((prev) => ({
        ...prev,
        smtpSecurity: security,
        smtpPort: getDefaultSmtpPort(security),
      }));
    },
    [],
  );

  const isOAuth = form.authMode === "oauth2";
  const hasOAuthTokens = Boolean(
    form.oauthAccessToken && form.oauthRefreshToken,
  );

  const canAdvanceFromBasic =
    form.email.trim().includes("@") &&
    (isOAuth ? hasOAuthTokens : form.password.trim().length > 0);
  const canAdvanceFromImap =
    form.imapHost.trim().length > 0 && form.imapPort > 0;
  const canAdvanceFromSmtp =
    form.smtpHost.trim().length > 0 && form.smtpPort > 0;
  const bothTestsPassed =
    imapTest.state === "success" && smtpTest.state === "success";

  const goNext = useCallback((): void => {
    const idx = steps.indexOf(currentStep);
    const nextStep = steps[idx + 1];
    if (idx < steps.length - 1 && nextStep) {
      setCurrentStep(nextStep);
    }
  }, [currentStep]);

  const goPrev = useCallback((): void => {
    const idx = steps.indexOf(currentStep);
    const prevStep = steps[idx - 1];
    if (idx > 0 && prevStep) {
      setCurrentStep(prevStep);
    } else {
      onBack();
    }
  }, [currentStep, onBack]);

  const canGoNextValue =
    currentStep === "basic"
      ? canAdvanceFromBasic
      : currentStep === "imap"
        ? canAdvanceFromImap
        : currentStep === "smtp"
          ? canAdvanceFromSmtp
          : false;

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent): void => {
      if (e.key === "Enter" && currentStep !== "test" && canGoNextValue) {
        e.preventDefault();
        goNext();
      }
    },
    [currentStep, goNext, canGoNextValue],
  );

  const handleOAuthConnect = async (providerId: string): Promise<void> => {
    const provider = getOAuthProvider(providerId);
    if (!provider) {
      setOauthError(`Unknown provider: ${providerId}`);
      return;
    }

    if (!form.oauthClientId.trim()) {
      setOauthError("Please enter a Client ID first.");
      return;
    }

    setOauthConnecting(true);
    setOauthError(null);

    try {
      const result = await invoke<OAuthAuthorizationResult>(
        "account_authorize_oauth_provider",
        {
          request: {
            providerId: provider.id,
            authUrl: provider.authUrl,
            tokenUrl: provider.tokenUrl,
            scopes: provider.scopes,
            userInfoUrl: provider.userInfoUrl ?? null,
            usePkce: provider.usePkce,
            clientId: form.oauthClientId.trim(),
            clientSecret: form.oauthClientSecret.trim() || null,
          },
        },
      );

      const expiresAt = Math.floor(Date.now() / 1000) + result.expiresIn;

      setForm((prev) => ({
        ...prev,
        oauthAccessToken: result.accessToken,
        oauthRefreshToken: result.refreshToken ?? null,
        oauthExpiresAt: expiresAt,
        oauthEmail: result.email,
        oauthPicture: result.picture ?? null,
        email: result.email || prev.email,
        displayName: result.name || prev.displayName,
        oauthProvider: providerId,
      }));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setOauthError(message);
    } finally {
      setOauthConnecting(false);
    }
  };

  const testImapConnection = async (): Promise<void> => {
    setImapTest({ state: "testing" });
    try {
      const result = await invoke<string>("imap_test_connection", {
        config: {
          host: form.imapHost,
          port: form.imapPort,
          security: mapSecurity(form.imapSecurity),
          username:
            form.imapUsername ||
            (isOAuth ? (form.oauthEmail ?? form.email) : form.email),
          password: isOAuth ? (form.oauthAccessToken ?? "") : form.password,
          auth_method: isOAuth ? "oauth2" : "password",
          accept_invalid_certs: form.acceptInvalidCerts,
        },
      });
      setImapTest({ state: "success", message: result });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setImapTest({ state: "error", message });
    }
  };

  const testSmtpConnection = async (): Promise<void> => {
    setSmtpTest({ state: "testing" });
    try {
      const smtpPassword = isOAuth
        ? (form.oauthAccessToken ?? "")
        : form.samePassword
          ? form.password
          : form.smtpPassword;
      const result = await invoke<{ success: boolean; message: string }>(
        "smtp_test_connection",
        {
          config: {
            host: form.smtpHost,
            port: form.smtpPort,
            security: mapSecurity(form.smtpSecurity),
            username:
              form.imapUsername ||
              (isOAuth ? (form.oauthEmail ?? form.email) : form.email),
            password: smtpPassword,
            auth_method: isOAuth ? "oauth2" : "password",
            accept_invalid_certs: form.acceptInvalidCerts,
          },
        },
      );
      setSmtpTest({
        state: result.success ? "success" : "error",
        message: result.message,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setSmtpTest({ state: "error", message });
    }
  };

  const testBothConnections = async (): Promise<void> => {
    await Promise.all([testImapConnection(), testSmtpConnection()]);
  };

  const handleSave = async (): Promise<void> => {
    setSaving(true);
    setSaveError(null);
    try {
      const email = (isOAuth ? form.oauthEmail : null) ?? form.email.trim();

      const imapUsername = form.imapUsername.trim() || null;
      let accountId: string;
      let avatarUrl: string | null = isOAuth ? form.oauthPicture : null;

      if (isOAuth) {
        const account = await invoke<{
          id: string;
          email: string;
          displayName: string;
          avatarUrl: string | null;
          isActive: boolean;
          provider: string;
        }>("account_create_imap_oauth", {
          request: {
            email,
            displayName: form.displayName.trim() || null,
            avatarUrl,
            imapHost: form.imapHost.trim(),
            imapPort: form.imapPort,
            imapSecurity: form.imapSecurity,
            smtpHost: form.smtpHost.trim(),
            smtpPort: form.smtpPort,
            smtpSecurity: form.smtpSecurity,
            accessToken: form.oauthAccessToken ?? "",
            refreshToken: form.oauthRefreshToken ?? "",
            tokenExpiresAt: form.oauthExpiresAt ?? 0,
            oauthProvider: form.oauthProvider ?? "",
            oauthClientId: form.oauthClientId.trim(),
            oauthClientSecret: form.oauthClientSecret.trim() || null,
            oauthTokenUrl:
              getOAuthProvider(form.oauthProvider ?? "")?.tokenUrl ?? null,
            imapUsername,
            acceptInvalidCerts: form.acceptInvalidCerts,
          },
        });
        accountId = account.id;
        avatarUrl = account.avatarUrl;
      } else {
        accountId = crypto.randomUUID();
        await insertImapAccount({
          id: accountId,
          email,
          displayName: form.displayName.trim() || null,
          avatarUrl: null,
          imapHost: form.imapHost.trim(),
          imapPort: form.imapPort,
          imapSecurity: form.imapSecurity,
          smtpHost: form.smtpHost.trim(),
          smtpPort: form.smtpPort,
          smtpSecurity: form.smtpSecurity,
          authMethod: "password",
          password: form.samePassword ? form.password : form.smtpPassword,
          imapUsername,
          acceptInvalidCerts: form.acceptInvalidCerts,
        });
      }

      addAccount({
        id: accountId,
        email,
        displayName: form.displayName.trim() || null,
        avatarUrl,
        isActive: true,
        provider: "imap",
      });

      onSuccess();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setSaveError(message);
      setSaving(false);
    }
  };

  const renderStepContent = (): React.ReactNode => {
    switch (currentStep) {
      case "basic":
        return (
          <AddImapAccountBasicStep
            form={form}
            updateForm={updateForm}
            handleEmailBlur={handleEmailBlur}
            isOAuth={isOAuth}
            hasOAuthTokens={hasOAuthTokens}
            detectedAuthMethods={detectedAuthMethods}
            detectedOAuthProviderId={detectedOAuthProviderId}
            oauthConnecting={oauthConnecting}
            oauthError={oauthError}
            onOAuthConnect={handleOAuthConnect}
          />
        );
      case "imap":
        return (
          <AddImapAccountImapStep
            form={form}
            updateForm={updateForm}
            isOAuth={isOAuth}
            onImapSecurityChange={handleImapSecurityChange}
          />
        );
      case "smtp":
        return (
          <AddImapAccountSmtpStep
            form={form}
            updateForm={updateForm}
            isOAuth={isOAuth}
            onSmtpSecurityChange={handleSmtpSecurityChange}
          />
        );
      case "test":
        return (
          <AddImapAccountTestStep
            imapTest={imapTest}
            smtpTest={smtpTest}
            saveError={saveError}
            onTestBoth={testBothConnections}
          />
        );
    }
  };

  return (
    <Modal
      isOpen={true}
      onClose={onClose}
      title={t("addImapAccount")}
      width="w-full max-w-lg"
    >
      {/* biome-ignore lint/a11y/noStaticElementInteractions: keyboard handler for form navigation */}
      <div className="p-4" onKeyDown={handleKeyDown}>
        <StepIndicator steps={stepDefs} currentStepIndex={currentStepIndex} />
        {renderStepContent()}

        <div className="flex items-center justify-between mt-6">
          <button
            type="button"
            onClick={goPrev}
            className="flex items-center gap-1 px-3 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            <ArrowLeft className="w-3.5 h-3.5" />
            {t("common:back")}
          </button>

          <div className="flex gap-2">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
            >
              {t("common:cancel")}
            </button>

            {currentStep === "test" ? (
              <button
                type="button"
                onClick={handleSave}
                disabled={!bothTestsPassed || saving}
                className="px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {saving ? t("common:adding") : t("addAccount")}
              </button>
            ) : (
              <button
                type="button"
                onClick={goNext}
                disabled={!canGoNextValue}
                className="flex items-center gap-1 px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {t("common:next")}
                <ArrowRight className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}
