import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import type { TestStatus } from "./addImapAccountTypes";

export interface AddImapAccountTestStepProps {
  imapTest: TestStatus;
  smtpTest: TestStatus;
  saveError: string | null;
  onTestBoth: () => void;
}

function renderTestResult(label: string, status: TestStatus): React.ReactNode {
  const icon =
    status.state === "testing" ? (
      <Loader2 className="w-4 h-4 animate-spin text-accent" />
    ) : status.state === "success" ? (
      <CheckCircle2 className="w-4 h-4 text-success" />
    ) : status.state === "error" ? (
      <XCircle className="w-4 h-4 text-danger" />
    ) : (
      <div className="w-4 h-4 rounded-full border-2 border-border-primary" />
    );

  return (
    <div className="flex items-start gap-3 p-3 rounded-lg bg-bg-secondary border border-border-primary">
      <div className="mt-0.5">{icon}</div>
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium text-text-primary">{label}</div>
        {status.message != null && (
          <div
            className={`text-xs mt-0.5 ${
              status.state === "error"
                ? "text-danger"
                : status.state === "success"
                  ? "text-success"
                  : "text-text-tertiary"
            }`}
          >
            {status.message}
          </div>
        )}
      </div>
    </div>
  );
}

export function AddImapAccountTestStep({
  imapTest,
  smtpTest,
  saveError,
  onTestBoth,
}: AddImapAccountTestStepProps): React.ReactNode {
  const { t } = useTranslation("accounts");

  return (
    <div className="space-y-4">
      <div className="text-sm text-text-secondary mb-2">
        {t("testDescription")}
      </div>

      <div className="space-y-3">
        {renderTestResult(t("imapConnection"), imapTest)}
        {renderTestResult(t("smtpConnection"), smtpTest)}
      </div>

      <button
        type="button"
        onClick={onTestBoth}
        disabled={imapTest.state === "testing" || smtpTest.state === "testing"}
        className="w-full px-4 py-2 text-sm bg-bg-secondary border border-border-primary rounded-lg text-text-primary hover:bg-bg-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {imapTest.state === "testing" || smtpTest.state === "testing"
          ? t("common:testing")
          : imapTest.state === "idle" && smtpTest.state === "idle"
            ? t("testConnection")
            : t("reTestConnection")}
      </button>

      {saveError != null && (
        <div className="bg-danger/10 border border-danger/20 rounded-lg p-3 text-sm text-danger">
          {saveError}
        </div>
      )}
    </div>
  );
}
