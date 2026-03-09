import { Mail, RefreshCw } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";
import { TextField } from "@/components/ui/TextField";
import {
  getAliasesForAccount,
  mapDbAlias,
  type SendAsAlias,
  setDefaultAlias,
} from "@/services/db/sendAsAliases";
import { setSetting } from "@/services/db/settings";
import { useAccountStore } from "@/stores/accountStore";
import { Section, SettingRow } from "./SettingsShared";

export interface SettingsAccountsTabProps {
  clientId: string;
  setClientId: (val: string) => void;
  clientSecret: string;
  setClientSecret: (val: string) => void;
  apiSettingsSaved: boolean;
  handleSaveApiSettings: () => Promise<void>;
  isSyncing: boolean;
  handleManualSync: () => Promise<void>;
  handleForceFullSync: () => Promise<void>;
  syncPeriodDays: string;
  setSyncPeriodDays: (val: string) => void;
  handleRemoveAccount: (accountId: string) => Promise<void>;
  handleReauthorizeAccount: (accountId: string, email: string) => Promise<void>;
  handleResyncAccount: (accountId: string) => Promise<void>;
  reauthStatus: Record<string, "idle" | "authorizing" | "done" | "error">;
  resyncStatus: Record<string, "idle" | "syncing" | "done" | "error">;
}

export function SettingsAccountsTab({
  clientId,
  setClientId,
  clientSecret,
  setClientSecret,
  apiSettingsSaved,
  handleSaveApiSettings,
  isSyncing,
  handleManualSync,
  handleForceFullSync,
  syncPeriodDays,
  setSyncPeriodDays,
  handleRemoveAccount,
  handleReauthorizeAccount,
  handleResyncAccount,
  reauthStatus,
  resyncStatus,
}: SettingsAccountsTabProps): React.ReactNode {
  const { t } = useTranslation("settings");
  const accounts = useAccountStore((s) => s.accounts);

  return (
    <>
      <Section title={t("mailAccounts")}>
        {accounts.filter((a) => a.provider !== "caldav").length === 0 ? (
          <p className="text-sm text-text-tertiary">{t("noMailAccounts")}</p>
        ) : (
          <div className="space-y-2">
            {accounts
              .filter((a) => a.provider !== "caldav")
              .map((account) => {
                const providerLabel =
                  account.provider === "imap" ? t("imap") : t("gmail");
                return (
                  <div
                    key={account.id}
                    className="flex items-center justify-between py-2.5 px-4 bg-bg-secondary rounded-lg"
                  >
                    <div>
                      <div className="text-sm font-medium text-text-primary flex items-center gap-2">
                        {account.displayName ?? account.email}
                        <span className="text-[0.6rem] font-medium px-1.5 py-0.5 rounded-full bg-bg-tertiary text-text-tertiary">
                          {providerLabel}
                        </span>
                      </div>
                      <div className="text-xs text-text-tertiary">
                        {account.email}
                      </div>
                    </div>
                    <div className="flex items-center gap-3">
                      <button
                        type="button"
                        onClick={(): void =>
                          void handleReauthorizeAccount(
                            account.id,
                            account.email,
                          )
                        }
                        disabled={reauthStatus[account.id] === "authorizing"}
                        className="text-xs text-accent hover:text-accent-hover transition-colors disabled:opacity-50"
                      >
                        {reauthStatus[account.id] === "authorizing" &&
                          t("waiting")}
                        {reauthStatus[account.id] === "done" && t("done")}
                        {reauthStatus[account.id] === "error" && t("failed")}
                        {(!reauthStatus[account.id] ||
                          reauthStatus[account.id] === "idle") &&
                          t("reauthorize")}
                      </button>
                      <button
                        type="button"
                        onClick={(): void =>
                          void handleResyncAccount(account.id)
                        }
                        disabled={resyncStatus[account.id] === "syncing"}
                        className="text-xs text-accent hover:text-accent-hover transition-colors disabled:opacity-50"
                      >
                        {resyncStatus[account.id] === "syncing" &&
                          t("resyncing")}
                        {resyncStatus[account.id] === "done" && t("done")}
                        {resyncStatus[account.id] === "error" && t("failed")}
                        {(!resyncStatus[account.id] ||
                          resyncStatus[account.id] === "idle") &&
                          t("resync")}
                      </button>
                      <button
                        type="button"
                        onClick={(): void =>
                          void handleRemoveAccount(account.id)
                        }
                        className="text-xs text-danger hover:text-danger/80 transition-colors"
                      >
                        {t("remove")}
                      </button>
                    </div>
                  </div>
                );
              })}
          </div>
        )}
      </Section>

      {accounts.some((a) => a.provider === "caldav") && (
        <Section title={t("calendarAccounts")}>
          <div className="space-y-2">
            {accounts
              .filter((a) => a.provider === "caldav")
              .map((account) => (
                <div
                  key={account.id}
                  className="flex items-center justify-between py-2.5 px-4 bg-bg-secondary rounded-lg"
                >
                  <div>
                    <div className="text-sm font-medium text-text-primary flex items-center gap-2">
                      {account.displayName ?? account.email}
                      <span className="text-[0.6rem] font-medium px-1.5 py-0.5 rounded-full bg-accent/10 text-accent">
                        {t("caldav")}
                      </span>
                    </div>
                    <div className="text-xs text-text-tertiary">
                      {account.email}
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={(): void => void handleRemoveAccount(account.id)}
                    className="text-xs text-danger hover:text-danger/80 transition-colors"
                  >
                    {t("remove")}
                  </button>
                </div>
              ))}
          </div>
        </Section>
      )}

      <SendAsAliasesSection />

      <ImapCalDavSection />

      <Section title={t("googleApi")}>
        <div className="space-y-3">
          <TextField
            label={t("clientId")}
            size="md"
            type="text"
            value={clientId}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setClientId(e.target.value)
            }
            placeholder={t("googleClientId")}
          />
          <TextField
            label={t("clientSecret")}
            size="md"
            type="password"
            value={clientSecret}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setClientSecret(e.target.value)
            }
            placeholder={t("googleClientSecret")}
          />
          <Button
            variant="primary"
            size="md"
            onClick={handleSaveApiSettings}
            disabled={!clientId.trim()}
          >
            {apiSettingsSaved ? t("saved") : t("save")}
          </Button>
        </div>
      </Section>

      <Section title={t("sync")}>
        <div className="flex items-center justify-between">
          <span className="text-sm text-text-secondary">
            {t("checkForNewMail")}
          </span>
          <Button
            variant="primary"
            size="md"
            icon={
              <RefreshCw
                size={14}
                className={isSyncing ? "animate-spin" : ""}
              />
            }
            onClick={handleManualSync}
            disabled={isSyncing || accounts.length === 0}
          >
            {isSyncing ? t("syncing") : t("syncNow")}
          </Button>
        </div>
        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm text-text-secondary">
              {t("fullResync")}
            </span>
            <p className="text-xs text-text-tertiary mt-0.5">
              {t("fullResyncDescription")}
            </p>
          </div>
          <Button
            variant="secondary"
            size="md"
            icon={
              <RefreshCw
                size={14}
                className={isSyncing ? "animate-spin" : ""}
              />
            }
            onClick={handleForceFullSync}
            disabled={isSyncing || accounts.length === 0}
            className="bg-bg-tertiary text-text-primary border border-border-primary"
          >
            {isSyncing ? t("syncing") : t("fullResync")}
          </Button>
        </div>
      </Section>

      <Section title={t("syncPeriod")}>
        <SettingRow label={t("syncEmailsFrom")}>
          <select
            value={syncPeriodDays}
            onChange={async (
              e: React.ChangeEvent<HTMLSelectElement>,
            ): Promise<void> => {
              const val = e.target.value;
              setSyncPeriodDays(val);
              await setSetting("sync_period_days", val);
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="30">{t("last30days")}</option>
            <option value="90">{t("last90days")}</option>
            <option value="180">{t("last180days")}</option>
            <option value="365">{t("last1year")}</option>
          </select>
        </SettingRow>
        <p className="text-xs text-text-tertiary">{t("syncPeriodNote")}</p>
      </Section>

      <SyncOfflineSection />
    </>
  );
}

function SendAsAliasesSection(): React.ReactNode {
  const { t } = useTranslation("settings");
  const accounts = useAccountStore((s) => s.accounts);
  const [aliases, setAliases] = useState<SendAsAlias[]>([]);

  useEffect(() => {
    const activeAccount = accounts.find((a) => a.isActive);
    if (!activeAccount) return;
    let cancelled = false;
    getAliasesForAccount(activeAccount.id).then((dbAliases) => {
      if (cancelled) return;
      setAliases(dbAliases.map(mapDbAlias));
    });
    return (): void => {
      cancelled = true;
    };
  }, [accounts]);

  const activeAccount = accounts.find((a) => a.isActive);

  const handleSetDefault = async (alias: SendAsAlias): Promise<void> => {
    if (!activeAccount) return;
    await setDefaultAlias(activeAccount.id, alias.id);
    setAliases((prev) =>
      prev.map((a) => ({
        ...a,
        isDefault: a.id === alias.id,
      })),
    );
  };

  return (
    <Section title={t("sendAsAliases")}>
      <p className="text-xs text-text-tertiary mb-3">
        {t("sendAsDescription")}
      </p>
      {aliases.length === 0 ? (
        <p className="text-sm text-text-tertiary">{t("noAliases")}</p>
      ) : (
        <div className="space-y-2">
          {aliases.map((alias) => (
            <div
              key={alias.id}
              className="flex items-center justify-between py-2.5 px-4 bg-bg-secondary rounded-lg"
            >
              <div className="flex items-center gap-3 min-w-0">
                <Mail size={15} className="text-text-tertiary shrink-0" />
                <div className="min-w-0">
                  <div className="text-sm font-medium text-text-primary truncate">
                    {alias.displayName
                      ? `${alias.displayName} <${alias.email}>`
                      : alias.email}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5">
                    {alias.isPrimary === true && (
                      <span className="text-[0.625rem] bg-accent/15 text-accent px-1.5 py-0.5 rounded-full">
                        {t("primaryAlias")}
                      </span>
                    )}
                    {alias.isDefault === true && (
                      <span className="text-[0.625rem] bg-success/15 text-success px-1.5 py-0.5 rounded-full">
                        {t("defaultAlias")}
                      </span>
                    )}
                    {alias.verificationStatus !== "accepted" && (
                      <span className="text-[0.625rem] bg-warning/15 text-warning px-1.5 py-0.5 rounded-full">
                        {alias.verificationStatus}
                      </span>
                    )}
                  </div>
                </div>
              </div>
              {!alias.isDefault && (
                <button
                  onClick={(): void => void handleSetDefault(alias)}
                  type="button"
                  className="text-xs text-accent hover:text-accent-hover transition-colors shrink-0 ml-3"
                >
                  {t("setAsDefault")}
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function SyncOfflineSection(): React.ReactNode {
  const { t } = useTranslation("settings");
  const [pendingCount, setPendingCount] = useState(0);
  const [failedCount, setFailedCount] = useState(0);
  const [loading, setLoading] = useState(false);

  const loadCounts = useCallback(async (): Promise<void> => {
    const { getPendingOpsCount, getFailedOpsCount } = await import(
      "@/services/db/pendingOperations"
    );
    setPendingCount(await getPendingOpsCount());
    setFailedCount(await getFailedOpsCount());
  }, []);

  useEffect(() => {
    void loadCounts();
  }, [loadCounts]);

  const handleRetryFailed = async (): Promise<void> => {
    setLoading(true);
    try {
      const { retryFailedOperations } = await import(
        "@/services/db/pendingOperations"
      );
      await retryFailedOperations();
      await loadCounts();
    } finally {
      setLoading(false);
    }
  };

  const handleClearFailed = async (): Promise<void> => {
    setLoading(true);
    try {
      const { clearFailedOperations } = await import(
        "@/services/db/pendingOperations"
      );
      await clearFailedOperations();
      await loadCounts();
    } finally {
      setLoading(false);
    }
  };

  return (
    <Section title={t("syncOffline")}>
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm text-text-secondary">
              {t("pendingOperations")}
            </span>
            <p className="text-xs text-text-tertiary mt-0.5">
              {t("pendingOpsDescription")}
            </p>
          </div>
          <span className="text-sm font-mono text-text-primary">
            {pendingCount}
          </span>
        </div>

        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm text-text-secondary">
              {t("failedOperations")}
            </span>
            <p className="text-xs text-text-tertiary mt-0.5">
              {t("failedOpsDescription")}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm font-mono text-text-primary">
              {failedCount}
            </span>
            {failedCount > 0 && (
              <>
                <button
                  type="button"
                  onClick={(): void => void handleRetryFailed()}
                  disabled={loading}
                  className="text-xs text-accent hover:text-accent-hover transition-colors disabled:opacity-50"
                >
                  {t("retry")}
                </button>
                <button
                  type="button"
                  onClick={(): void => void handleClearFailed()}
                  disabled={loading}
                  className="text-xs text-danger hover:opacity-80 transition-colors disabled:opacity-50"
                >
                  {t("clear")}
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </Section>
  );
}

function ImapCalDavSection(): React.ReactNode {
  const { t } = useTranslation("settings");
  const accounts = useAccountStore((s) => s.accounts);
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const [account, setAccount] = useState<
    import("@/services/db/accounts").DbAccount | null
  >(null);

  useEffect(() => {
    if (!activeAccountId) return;
    void import("@/services/db/accounts").then(({ getAccount }) =>
      getAccount(activeAccountId).then(setAccount),
    );
  }, [activeAccountId]);

  const activeUiAccount = accounts.find((a) => a.id === activeAccountId);
  const isImap = activeUiAccount?.provider === "imap";

  if (!(isImap && account)) return null;

  return (
    <Section title={t("calendarCaldav")}>
      <CalDavSettingsInline
        account={account}
        onSaved={(): void => {
          // Reload account
          void import("@/services/db/accounts").then(({ getAccount }) =>
            getAccount(account.id).then(setAccount),
          );
        }}
      />
    </Section>
  );
}

function CalDavSettingsInline({
  account,
  onSaved,
}: {
  account: import("@/services/db/accounts").DbAccount;
  onSaved: () => void;
}): React.ReactNode {
  const { t } = useTranslation("settings");
  const [CalDav, setCalDav] = useState<
    typeof import("@/components/settings/CalDavSettings").CalDavSettings | null
  >(null);

  useEffect(() => {
    void import("@/components/settings/CalDavSettings").then((m) =>
      setCalDav(() => m.CalDavSettings),
    );
  }, []);

  if (!CalDav)
    return <div className="text-xs text-text-tertiary">{t("loading")}</div>;

  return <CalDav account={account} onSaved={onSaved} />;
}
