import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  ArrowRight,
  CheckCircle,
  Globe,
  Loader2,
  Mail,
  ShieldCheck,
  XCircle,
} from "lucide-react";
import type React from "react";
import { useCallback, useRef, useState } from "react";
import { Modal } from "@/components/ui/Modal";
import { StepIndicator, type StepDef } from "./StepIndicator";
import { deleteAccount, insertJmapAccount } from "@/services/db/accounts";
import { useAccountStore } from "@/stores/accountStore";

type Step = "credentials" | "discovery" | "test";

interface DiscoveryResult {
  sessionUrl: string;
  source: string;
}

interface TestResult {
  success: boolean;
  message: string;
}

type StepStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success"; message?: string }
  | { state: "error"; message: string };

interface AddJmapAccountProps {
  onComplete: () => void;
  onCancel: () => void;
}

const steps: Step[] = ["credentials", "discovery", "test"];

const stepLabels: Record<Step, string> = {
  credentials: "Credentials",
  discovery: "Discovery",
  test: "Verify",
};

const stepIcons: Record<Step, React.ReactNode> = {
  credentials: <Mail className="w-4 h-4" />,
  discovery: <Globe className="w-4 h-4" />,
  test: <ShieldCheck className="w-4 h-4" />,
};

export function AddJmapAccount({
  onComplete,
  onCancel,
}: AddJmapAccountProps): React.ReactNode {
  const [currentStep, setCurrentStep] = useState<Step>("credentials");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [username, setUsername] = useState("");
  const [jmapUrl, setJmapUrl] = useState("");
  const [discoveryStatus, setDiscoveryStatus] = useState<StepStatus>({
    state: "idle",
  });
  const [testStatus, setTestStatus] = useState<StepStatus>({ state: "idle" });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const accountIdRef = useRef<string | null>(null);
  const addAccount = useAccountStore((s) => s.addAccount);

  const currentStepIndex = steps.indexOf(currentStep);

  const stepDefs: StepDef[] = steps.map((s) => ({
    key: s,
    label: stepLabels[s],
    icon: stepIcons[s],
  }));

  const canAdvanceFromCredentials =
    email.trim().includes("@") && password.trim().length > 0;
  const canAdvanceFromDiscovery = jmapUrl.trim().length > 0;
  const testPassed = testStatus.state === "success";

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
      onCancel();
    }
  }, [currentStep, onCancel]);

  const canGoNextValue =
    currentStep === "credentials"
      ? canAdvanceFromCredentials
      : currentStep === "discovery"
        ? canAdvanceFromDiscovery
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

  const handleDiscover = async (): Promise<void> => {
    setDiscoveryStatus({ state: "loading" });
    try {
      const result = await invoke<DiscoveryResult | null>(
        "jmap_discover_url",
        { email: email.trim() },
      );
      if (result) {
        setJmapUrl(result.sessionUrl);
        setDiscoveryStatus({
          state: "success",
          message: `Found via ${result.source}`,
        });
      } else {
        setDiscoveryStatus({
          state: "error",
          message: "Could not auto-discover JMAP URL. Please enter it manually.",
        });
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setDiscoveryStatus({ state: "error", message });
    }
  };

  const cleanup = async (): Promise<void> => {
    if (accountIdRef.current) {
      try {
        await invoke("jmap_remove_client", {
          accountId: accountIdRef.current,
        });
      } catch {
        // Best-effort cleanup
      }
      try {
        await deleteAccount(accountIdRef.current);
      } catch {
        // Best-effort cleanup
      }
      accountIdRef.current = null;
    }
  };

  const handleTest = async (): Promise<void> => {
    setTestStatus({ state: "loading" });
    setSaveError(null);

    try {
      // Generate account ID and save to DB first so Rust can read credentials
      const accountId = crypto.randomUUID();
      accountIdRef.current = accountId;

      await insertJmapAccount({
        id: accountId,
        email: email.trim(),
        displayName: displayName.trim() || null,
        jmapUrl: jmapUrl.trim(),
        password: password.trim(),
        username: username.trim() || null,
      });

      // Initialize the JMAP client
      await invoke("jmap_init_client", { accountId });

      // Test the connection
      const result = await invoke<TestResult>("jmap_test_connection", {
        accountId,
      });

      if (result.success) {
        setTestStatus({ state: "success", message: result.message });
      } else {
        setTestStatus({ state: "error", message: result.message });
        await cleanup();
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setTestStatus({ state: "error", message });
      await cleanup();
    }
  };

  const handleSave = async (): Promise<void> => {
    setSaving(true);
    setSaveError(null);

    try {
      const accountId = accountIdRef.current;
      if (!accountId) {
        throw new Error("No account ID — please re-test the connection.");
      }

      addAccount({
        id: accountId,
        email: email.trim(),
        displayName: displayName.trim() || null,
        avatarUrl: null,
        isActive: true,
        provider: "jmap",
      });

      onComplete();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setSaveError(message);
      setSaving(false);
    }
  };

  const handleCancel = async (): Promise<void> => {
    await cleanup();
    onCancel();
  };

  const renderCredentialsStep = (): React.ReactNode => (
    <div className="space-y-4">
      <div>
        <label className="block text-xs font-medium text-text-secondary mb-1">
          Email Address
        </label>
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
          className="w-full px-3 py-2 text-sm bg-bg-secondary border border-border-primary rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent"
          autoFocus
        />
      </div>

      <div>
        <label className="block text-xs font-medium text-text-secondary mb-1">
          Password
        </label>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="Enter your password or app password"
          className="w-full px-3 py-2 text-sm bg-bg-secondary border border-border-primary rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <p className="mt-1 text-xs text-text-tertiary">
          If your provider requires it, use an app-specific password.
        </p>
      </div>

      <div>
        <label className="block text-xs font-medium text-text-secondary mb-1">
          Display Name (optional)
        </label>
        <input
          type="text"
          value={displayName}
          onChange={(e) => setDisplayName(e.target.value)}
          placeholder="Your Name"
          className="w-full px-3 py-2 text-sm bg-bg-secondary border border-border-primary rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent"
        />
      </div>

      <div>
        <label className="block text-xs font-medium text-text-secondary mb-1">
          Username (optional)
        </label>
        <input
          type="text"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          placeholder="Leave blank to use your email address"
          className="w-full px-3 py-2 text-sm bg-bg-secondary border border-border-primary rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <p className="mt-1 text-xs text-text-tertiary">
          Only needed if your login username differs from your email address.
        </p>
      </div>
    </div>
  );

  const renderDiscoveryStep = (): React.ReactNode => (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handleDiscover}
          disabled={discoveryStatus.state === "loading"}
          className="px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
        >
          {discoveryStatus.state === "loading" && (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          )}
          Auto-discover
        </button>
        {discoveryStatus.state === "success" && (
          <div className="flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
            <CheckCircle className="w-3.5 h-3.5" />
            {discoveryStatus.message}
          </div>
        )}
        {discoveryStatus.state === "error" && (
          <div className="flex items-center gap-1 text-xs text-red-500">
            <XCircle className="w-3.5 h-3.5" />
            {discoveryStatus.message}
          </div>
        )}
      </div>

      <div>
        <label className="block text-xs font-medium text-text-secondary mb-1">
          JMAP Session URL
        </label>
        <input
          type="url"
          value={jmapUrl}
          onChange={(e) => setJmapUrl(e.target.value)}
          placeholder="https://jmap.example.com/.well-known/jmap"
          className="w-full px-3 py-2 text-sm bg-bg-secondary border border-border-primary rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <p className="mt-1 text-xs text-text-tertiary">
          The JMAP session URL for your mail server. Try auto-discover first, or
          enter it manually.
        </p>
      </div>
    </div>
  );

  const renderTestStep = (): React.ReactNode => (
    <div className="space-y-4">
      <p className="text-sm text-text-secondary">
        Test your connection settings before adding the account.
      </p>

      <div className="flex items-center gap-3 p-3 rounded-lg border border-border-primary bg-bg-secondary">
        <div className="flex-1">
          <div className="text-sm font-medium text-text-primary">
            JMAP Connection
          </div>
          {testStatus.state === "idle" && (
            <div className="text-xs text-text-tertiary">Not tested yet</div>
          )}
          {testStatus.state === "loading" && (
            <div className="flex items-center gap-1.5 text-xs text-text-secondary">
              <Loader2 className="w-3 h-3 animate-spin" />
              Testing connection...
            </div>
          )}
          {testStatus.state === "success" && (
            <div className="flex items-center gap-1.5 text-xs text-green-600 dark:text-green-400">
              <CheckCircle className="w-3 h-3" />
              {testStatus.message ?? "Connection successful"}
            </div>
          )}
          {testStatus.state === "error" && (
            <div className="flex items-center gap-1.5 text-xs text-red-500">
              <XCircle className="w-3 h-3" />
              {testStatus.message}
            </div>
          )}
        </div>
      </div>

      <button
        type="button"
        onClick={handleTest}
        disabled={testStatus.state === "loading"}
        className="px-4 py-2 text-sm bg-accent/10 text-accent rounded-lg hover:bg-accent/20 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {testStatus.state === "idle" ? "Test Connection" : "Re-test Connection"}
      </button>

      {saveError && (
        <div className="p-3 rounded-lg bg-red-500/10 border border-red-500/20 text-xs text-red-500">
          {saveError}
        </div>
      )}
    </div>
  );

  const renderStepContent = (): React.ReactNode => {
    switch (currentStep) {
      case "credentials":
        return renderCredentialsStep();
      case "discovery":
        return renderDiscoveryStep();
      case "test":
        return renderTestStep();
    }
  };

  return (
    <Modal
      isOpen={true}
      onClose={handleCancel}
      title="Add JMAP Account"
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
            Back
          </button>

          <div className="flex gap-2">
            <button
              type="button"
              onClick={handleCancel}
              className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
            >
              Cancel
            </button>

            {currentStep === "test" ? (
              <button
                type="button"
                onClick={handleSave}
                disabled={!testPassed || saving}
                className="px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {saving ? "Adding..." : "Add Account"}
              </button>
            ) : (
              <button
                type="button"
                onClick={goNext}
                disabled={!canGoNextValue}
                className="flex items-center gap-1 px-4 py-2 text-sm bg-accent text-white rounded-lg hover:bg-accent-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Next
                <ArrowRight className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}
