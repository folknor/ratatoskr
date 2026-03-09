import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";
import { TextField } from "@/components/ui/TextField";
import {
  discoverCalDavSettings,
  testCalDavConnection,
  removeCalendarProvider,
} from "@/core/calendar";
import { type DbAccount, updateAccountCalDav } from "@/core/accounts";

interface CalDavSettingsProps {
  account: DbAccount;
  onSaved: () => void;
}

export function CalDavSettings({
  account,
  onSaved,
}: CalDavSettingsProps): React.ReactNode {
  const { t } = useTranslation("settings");
  const [caldavUrl, setCaldavUrl] = useState(account.caldav_url ?? "");
  const [username, setUsername] = useState(
    account.caldav_username ?? account.email,
  );
  const [password, setPassword] = useState(account.caldav_password ?? "");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{
    success: boolean;
    message: string;
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const [discovered, setDiscovered] = useState(false);

  // Auto-discover on mount if not already configured
  useEffect(() => {
    if (!(account.caldav_url || discovered)) {
      setDiscovered(true);
      void discoverCalDavSettings(account.email).then((result) => {
        if (result.caldavUrl) {
          setCaldavUrl(result.caldavUrl);
        }
      });
    }
  }, [account.email, account.caldav_url, discovered]);

  const handleTest = useCallback(async (): Promise<void> => {
    setTesting(true);
    setTestResult(null);
    const result = await testCalDavConnection(caldavUrl, username, password);
    setTestResult(result);
    setTesting(false);
  }, [caldavUrl, username, password]);

  const handleSave = useCallback(async (): Promise<void> => {
    setSaving(true);
    try {
      await updateAccountCalDav(account.id, {
        caldavUrl,
        caldavUsername: username,
        caldavPassword: password,
        calendarProvider: "caldav",
      });
      removeCalendarProvider(account.id);
      onSaved();
    } catch (err) {
      console.error("Failed to save CalDAV settings:", err);
    } finally {
      setSaving(false);
    }
  }, [account.id, caldavUrl, username, password, onSaved]);

  const handleRemove = useCallback(async (): Promise<void> => {
    setSaving(true);
    try {
      await updateAccountCalDav(account.id, {
        caldavUrl: "",
        caldavUsername: "",
        caldavPassword: "",
        calendarProvider: "",
      });
      removeCalendarProvider(account.id);
      setCaldavUrl("");
      setUsername(account.email);
      setPassword("");
      setTestResult(null);
      onSaved();
    } finally {
      setSaving(false);
    }
  }, [account.id, account.email, onSaved]);

  const isConfigured = Boolean(account.caldav_url);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium text-text-primary">
          {t("caldavEditor.heading")}
        </h4>
        {isConfigured === true && (
          <span className="text-xs text-success font-medium">
            {t("caldavEditor.connected")}
          </span>
        )}
      </div>
      <p className="text-xs text-text-tertiary">
        {t("caldavEditor.description")}
      </p>

      <TextField
        label={t("caldavEditor.serverUrl")}
        type="url"
        value={caldavUrl}
        onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
          setCaldavUrl(e.target.value)
        }
        placeholder={t("caldavEditor.serverUrlPlaceholder")}
      />

      <TextField
        label={t("caldavEditor.username")}
        type="text"
        value={username}
        onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
          setUsername(e.target.value)
        }
        placeholder={t("caldavEditor.usernamePlaceholder")}
      />

      <TextField
        label={t("caldavEditor.password")}
        type="password"
        value={password}
        onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
          setPassword(e.target.value)
        }
        placeholder={t("caldavEditor.passwordPlaceholder")}
      />

      {testResult != null && (
        <div
          className={`flex items-center gap-2 text-xs ${testResult.success ? "text-success" : "text-danger"}`}
        >
          {testResult.success ? (
            <CheckCircle2 size={14} />
          ) : (
            <XCircle size={14} />
          )}
          {testResult.message}
        </div>
      )}

      <div className="flex items-center gap-2">
        <Button
          variant="secondary"
          size="sm"
          onClick={handleTest}
          disabled={testing || !caldavUrl || !password}
        >
          {testing === true && <Loader2 size={14} className="animate-spin" />}
          {testing
            ? t("caldavEditor.testing")
            : t("caldavEditor.testConnection")}
        </Button>

        <Button
          variant="primary"
          size="sm"
          onClick={handleSave}
          disabled={saving || !caldavUrl || !password}
        >
          {saving ? t("caldavEditor.saving") : t("caldavEditor.save")}
        </Button>

        {isConfigured === true && (
          <Button
            variant="ghost"
            size="sm"
            onClick={handleRemove}
            disabled={saving}
          >
            {t("caldavEditor.remove")}
          </Button>
        )}
      </div>
    </div>
  );
}
