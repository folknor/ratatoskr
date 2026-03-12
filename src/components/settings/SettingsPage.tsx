import { useParams } from "@tanstack/react-router";
import {
  ArrowLeft,
  Bell,
  Filter,
  Info,
  Keyboard,
  type LucideIcon,
  PenLine,
  Settings,
  Sparkles,
  UserCircle,
  Users,
} from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteAccount } from "@/core/accounts";
import { setSecureSetting, setSetting } from "@/core/settings";
import {
  forceFullSync,
  reauthorizeAccount,
  removeClient,
  resyncAccount,
  triggerSync,
} from "@/core/sync";
import { getPersistedLanguage, getSystemLanguageName } from "@/i18n";
import { navigateToLabel, navigateToSettings } from "@/router/navigate";
import {
  getSettingsBootstrapSnapshot,
  getSettingsSecretsSnapshot,
} from "@/services/settings/bootstrapSnapshot";
import { useAccountStore } from "@/stores/accountStore";
import { SettingsAboutTab } from "./SettingsAboutTab";
import { SettingsAccountsTab } from "./SettingsAccountsTab";
import { SettingsAiTab } from "./SettingsAiTab";
import { SettingsComposingTab } from "./SettingsComposingTab";
import { SettingsGeneralTab } from "./SettingsGeneralTab";
import { SettingsMailRulesTab } from "./SettingsMailRulesTab";
import { SettingsNotificationsTab } from "./SettingsNotificationsTab";
import { SettingsPeopleTab } from "./SettingsPeopleTab";
import { SettingsShortcutsTab } from "./SettingsShortcutsTab";

type SettingsTab =
  | "general"
  | "notifications"
  | "composing"
  | "mail-rules"
  | "people"
  | "accounts"
  | "shortcuts"
  | "ai"
  | "about";

const TAB_ICONS: Record<SettingsTab, LucideIcon> = {
  general: Settings,
  notifications: Bell,
  composing: PenLine,
  "mail-rules": Filter,
  people: Users,
  accounts: UserCircle,
  shortcuts: Keyboard,
  ai: Sparkles,
  about: Info,
};

const TAB_IDS: SettingsTab[] = [
  "general",
  "notifications",
  "composing",
  "mail-rules",
  "people",
  "accounts",
  "shortcuts",
  "ai",
  "about",
];

const TAB_LABEL_KEYS = {
  general: "tabGeneral",
  notifications: "tabNotifications",
  composing: "tabComposing",
  "mail-rules": "tabMailRules",
  people: "tabPeople",
  accounts: "tabAccounts",
  shortcuts: "tabShortcuts",
  ai: "tabAi",
  about: "tabAbout",
} as const;

export function SettingsPage(): React.ReactNode {
  const { t } = useTranslation("settings");
  const accounts = useAccountStore((s) => s.accounts);
  const removeAccountFromStore = useAccountStore((s) => s.removeAccount);
  const { tab } = useParams({ strict: false }) as { tab?: string };
  const activeTab = (
    tab && TAB_IDS.includes(tab as SettingsTab) ? tab : "general"
  ) as SettingsTab;
  const setActiveTab = (tabId: SettingsTab): void => navigateToSettings(tabId);
  const [languageOverride, setLanguageOverride] = useState<string | null>(null);
  const [languageLoaded, setLanguageLoaded] = useState(false);
  const [systemLanguageName, setSystemLanguageName] = useState("English");
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [undoSendDelay, setUndoSendDelay] = useState("5");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [microsoftClientId, setMicrosoftClientId] = useState("");
  const [apiSettingsSaved, setApiSettingsSaved] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [syncPeriodDays, setSyncPeriodDays] = useState("365");
  const [blockRemoteImages, setBlockRemoteImages] = useState(true);
  const [phishingDetectionEnabled, setPhishingDetectionEnabled] =
    useState(true);
  const [phishingSensitivity, setPhishingSensitivity] = useState<
    "low" | "default" | "high"
  >("default");
  const [aiProvider, setAiProvider] = useState<
    "claude" | "openai" | "gemini" | "ollama" | "copilot"
  >("claude");
  const [claudeApiKey, setClaudeApiKey] = useState("");
  const [openaiApiKey, setOpenaiApiKey] = useState("");
  const [geminiApiKey, setGeminiApiKey] = useState("");
  const [copilotApiKey, setCopilotApiKey] = useState("");
  const [ollamaServerUrl, setOllamaServerUrl] = useState(
    "http://localhost:11434",
  );
  const [ollamaModel, setOllamaModel] = useState("llama3.2");
  const [claudeModel, setClaudeModel] = useState("claude-haiku-4-5-20251001");
  const [openaiModel, setOpenaiModel] = useState("gpt-4o-mini");
  const [geminiModel, setGeminiModel] = useState(
    "gemini-2.5-flash-preview-05-20",
  );
  const [copilotModel, setCopilotModel] = useState("openai/gpt-4o-mini");
  const [aiEnabled, setAiEnabled] = useState(true);
  const [aiAutoCategorize, setAiAutoCategorize] = useState(true);
  const [aiAutoSummarize, setAiAutoSummarize] = useState(true);
  const [aiKeySaved, setAiKeySaved] = useState(false);
  const [aiTesting, setAiTesting] = useState(false);
  const [aiTestResult, setAiTestResult] = useState<"success" | "fail" | null>(
    null,
  );
  const [aiAutoDraftEnabled, setAiAutoDraftEnabled] = useState(true);
  const [aiWritingStyleEnabled, setAiWritingStyleEnabled] = useState(true);
  const [styleAnalyzing, setStyleAnalyzing] = useState(false);
  const [styleAnalyzeDone, setStyleAnalyzeDone] = useState(false);
  const [cacheMaxMb, setCacheMaxMb] = useState("500");
  const [cacheSizeMb, setCacheSizeMb] = useState<number | null>(null);
  const [clearingCache, setClearingCache] = useState(false);
  const [reauthStatus, setReauthStatus] = useState<
    Record<string, "idle" | "authorizing" | "done" | "error">
  >({});
  const [resyncStatus, setResyncStatus] = useState<
    Record<string, "idle" | "syncing" | "done" | "error">
  >({});
  const [autoArchiveCategories, setAutoArchiveCategories] = useState<
    Set<string>
  >(() => new Set());
  const [smartNotifications, setSmartNotifications] = useState(true);
  const [notifyCategories, setNotifyCategories] = useState<Set<string>>(
    () => new Set(["Primary"]),
  );
  const [vipSenders, setVipSenders] = useState<
    { email_address: string; display_name: string | null }[]
  >([]);
  const [newVipEmail, setNewVipEmail] = useState("");

  // Load settings from DB
  useEffect(() => {
    async function load(): Promise<void> {
      const [snapshot, secrets] = await Promise.all([
        getSettingsBootstrapSnapshot(),
        getSettingsSecretsSnapshot(),
      ]);

      setNotificationsEnabled(snapshot.notificationsEnabled);
      setUndoSendDelay(snapshot.undoSendDelaySeconds ?? "5");
      setClientId(snapshot.googleClientId ?? "");
      setClientSecret(secrets.googleClientSecret ?? "");
      setMicrosoftClientId(snapshot.microsoftClientId ?? "");
      setBlockRemoteImages(snapshot.blockRemoteImages);
      setPhishingDetectionEnabled(snapshot.phishingDetectionEnabled);
      if (
        snapshot.phishingSensitivity === "low" ||
        snapshot.phishingSensitivity === "high"
      ) {
        setPhishingSensitivity(snapshot.phishingSensitivity);
      }
      setSyncPeriodDays(snapshot.syncPeriodDays ?? "365");

      // Load AI settings
      const provider = snapshot.aiProvider;
      if (
        provider === "openai" ||
        provider === "gemini" ||
        provider === "ollama" ||
        provider === "copilot"
      )
        setAiProvider(provider);
      const ollamaUrl = snapshot.ollamaServerUrl;
      if (ollamaUrl) setOllamaServerUrl(ollamaUrl);
      const ollamaModelVal = snapshot.ollamaModel;
      if (ollamaModelVal) setOllamaModel(ollamaModelVal);
      const claudeModelVal = snapshot.claudeModel;
      if (claudeModelVal) setClaudeModel(claudeModelVal);
      const openaiModelVal = snapshot.openaiModel;
      if (openaiModelVal) setOpenaiModel(openaiModelVal);
      const geminiModelVal = snapshot.geminiModel;
      if (geminiModelVal) setGeminiModel(geminiModelVal);
      const aiKey = secrets.claudeApiKey;
      setClaudeApiKey(aiKey ?? "");
      const oaiKey = secrets.openaiApiKey;
      setOpenaiApiKey(oaiKey ?? "");
      const gemKey = secrets.geminiApiKey;
      setGeminiApiKey(gemKey ?? "");
      const copKey = secrets.copilotApiKey;
      setCopilotApiKey(copKey ?? "");
      const copilotModelVal = snapshot.copilotModel;
      if (copilotModelVal) setCopilotModel(copilotModelVal);
      setAiEnabled(snapshot.aiEnabled);
      setAiAutoCategorize(snapshot.aiAutoCategorize);
      setAiAutoSummarize(snapshot.aiAutoSummarize);
      setAiAutoDraftEnabled(snapshot.aiAutoDraftEnabled);
      setAiWritingStyleEnabled(snapshot.aiWritingStyleEnabled);

      // Load auto-archive categories
      const autoArchive = snapshot.autoArchiveCategories;
      if (autoArchive) {
        setAutoArchiveCategories(
          new Set(
            autoArchive
              .split(",")
              .map((s) => s.trim())
              .filter(Boolean),
          ),
        );
      }

      // Load smart notification settings
      setSmartNotifications(snapshot.smartNotifications);
      const notifCats = snapshot.notifyCategories;
      if (notifCats) {
        setNotifyCategories(
          new Set(
            notifCats
              .split(",")
              .map((s) => s.trim())
              .filter(Boolean),
          ),
        );
      }
      try {
        const { getAllVipSenders } = await import(
          "@/services/db/notificationVips"
        );
        const activeId = accounts.find((a) => a.isActive)?.id;
        if (activeId) {
          const vips = await getAllVipSenders(activeId);
          setVipSenders(
            vips.map((v) => ({
              email_address: v.email_address,
              display_name: v.display_name,
            })),
          );
        }
      } catch {
        // VIP table may not exist yet
      }

      // Load cache settings
      const cacheMax = snapshot.attachmentCacheMaxMb;
      setCacheMaxMb(cacheMax ?? "500");
      try {
        const { getCacheSize } = await import(
          "@/services/attachments/cacheManager"
        );
        const size = await getCacheSize();
        setCacheSizeMb(Math.round((size / (1024 * 1024)) * 10) / 10);
      } catch {
        // cache manager may not be available
      }

      // Load persisted language preference
      const persisted = await getPersistedLanguage();
      setLanguageOverride(persisted);
      const sysLang = await getSystemLanguageName();
      setSystemLanguageName(sysLang);
      setLanguageLoaded(true);
    }
    void load();
  }, [accounts.find]);

  const handleNotificationsToggle = useCallback(async (): Promise<void> => {
    const newVal = !notificationsEnabled;
    setNotificationsEnabled(newVal);
    await setSetting("notifications_enabled", newVal ? "true" : "false");
  }, [notificationsEnabled]);

  const handleUndoDelayChange = useCallback(
    async (value: string): Promise<void> => {
      setUndoSendDelay(value);
      await setSetting("undo_send_delay_seconds", value);
    },
    [],
  );

  const handleSaveApiSettings = useCallback(async (): Promise<void> => {
    const trimmedId = clientId.trim();
    if (trimmedId) {
      await setSetting("google_client_id", trimmedId);
    }
    const trimmedSecret = clientSecret.trim();
    if (trimmedSecret) {
      await setSecureSetting("google_client_secret", trimmedSecret);
    }
    setApiSettingsSaved(true);
    setTimeout(() => setApiSettingsSaved(false), 2000);
  }, [clientId, clientSecret]);

  const handleManualSync = useCallback(async (): Promise<void> => {
    const activeIds = accounts.filter((a) => a.isActive).map((a) => a.id);
    if (activeIds.length === 0) return;
    setIsSyncing(true);
    try {
      await triggerSync(activeIds);
    } finally {
      setIsSyncing(false);
    }
  }, [accounts]);

  const handleForceFullSync = useCallback(async (): Promise<void> => {
    const activeIds = accounts.filter((a) => a.isActive).map((a) => a.id);
    if (activeIds.length === 0) return;
    setIsSyncing(true);
    try {
      await forceFullSync(activeIds);
    } finally {
      setIsSyncing(false);
    }
  }, [accounts]);

  const handleRemoveAccount = useCallback(
    async (accountId: string): Promise<void> => {
      removeClient(accountId);
      await deleteAccount(accountId);
      removeAccountFromStore(accountId);
    },
    [removeAccountFromStore],
  );

  const handleReauthorizeAccount = useCallback(
    async (accountId: string, email: string): Promise<void> => {
      setReauthStatus((prev) => ({ ...prev, [accountId]: "authorizing" }));
      try {
        await reauthorizeAccount(accountId, email);
        setReauthStatus((prev) => ({ ...prev, [accountId]: "done" }));
        setTimeout(() => {
          setReauthStatus((prev) => ({ ...prev, [accountId]: "idle" }));
        }, 3000);
      } catch (err) {
        console.error("Re-authorization failed:", err);
        setReauthStatus((prev) => ({ ...prev, [accountId]: "error" }));
        setTimeout(() => {
          setReauthStatus((prev) => ({ ...prev, [accountId]: "idle" }));
        }, 3000);
      }
    },
    [],
  );

  const handleResyncAccount = useCallback(
    async (accountId: string): Promise<void> => {
      setResyncStatus((prev) => ({ ...prev, [accountId]: "syncing" }));
      try {
        await resyncAccount(accountId);
        setResyncStatus((prev) => ({ ...prev, [accountId]: "done" }));
        setTimeout(() => {
          setResyncStatus((prev) => ({ ...prev, [accountId]: "idle" }));
        }, 3000);
      } catch (err) {
        console.error("Resync failed:", err);
        setResyncStatus((prev) => ({ ...prev, [accountId]: "error" }));
        setTimeout(() => {
          setResyncStatus((prev) => ({ ...prev, [accountId]: "idle" }));
        }, 3000);
      }
    },
    [],
  );

  return (
    <div className="flex-1 flex flex-col min-w-0 overflow-hidden bg-bg-primary/50">
      {/* Header */}
      <div className="flex items-center gap-3 px-5 py-3 border-b border-border-primary shrink-0 bg-bg-primary/60 backdrop-blur-sm">
        <button
          type="button"
          onClick={(): void => navigateToLabel("inbox")}
          className="p-1.5 -ml-1 rounded-md text-text-secondary hover:text-text-primary hover:bg-bg-hover transition-colors"
          title={t("backToInbox")}
        >
          <ArrowLeft size={18} />
        </button>
        <h1 className="text-base font-semibold text-text-primary">
          {t("settings")}
        </h1>
      </div>

      {/* Body: sidebar nav + content */}
      <div className="flex flex-1 min-h-0">
        {/* Vertical tab sidebar */}
        <nav className="w-48 border-r border-border-primary py-2 overflow-y-auto shrink-0 bg-bg-primary/30">
          {TAB_IDS.map((tabId) => {
            const Icon = TAB_ICONS[tabId];
            const isActive = activeTab === tabId;
            return (
              <button
                type="button"
                key={tabId}
                onClick={(): void => setActiveTab(tabId)}
                className={`flex items-center gap-2.5 w-full px-4 py-2 text-[0.8125rem] transition-colors ${
                  isActive
                    ? "bg-bg-selected text-accent font-medium"
                    : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"
                }`}
              >
                <Icon size={15} className="shrink-0" />
                {t(TAB_LABEL_KEYS[tabId])}
              </button>
            );
          })}
        </nav>

        {/* Scrollable content */}
        <div className="flex-1 overflow-y-auto">
          <div className="max-w-2xl px-8 py-6">
            {/* Tab title */}
            <div className="mb-6">
              <h2 className="text-lg font-semibold text-text-primary">
                {t(TAB_LABEL_KEYS[activeTab])}
              </h2>
            </div>

            <div className="space-y-8">
              {activeTab === "general" && (
                <SettingsGeneralTab
                  languageOverride={languageOverride}
                  setLanguageOverride={setLanguageOverride}
                  languageLoaded={languageLoaded}
                  systemLanguageName={systemLanguageName}
                  blockRemoteImages={blockRemoteImages}
                  setBlockRemoteImages={setBlockRemoteImages}
                  phishingDetectionEnabled={phishingDetectionEnabled}
                  setPhishingDetectionEnabled={setPhishingDetectionEnabled}
                  phishingSensitivity={phishingSensitivity}
                  setPhishingSensitivity={setPhishingSensitivity}
                  cacheSizeMb={cacheSizeMb}
                  cacheMaxMb={cacheMaxMb}
                  setCacheMaxMb={setCacheMaxMb}
                  clearingCache={clearingCache}
                  setClearingCache={setClearingCache}
                  setCacheSizeMb={setCacheSizeMb}
                />
              )}

              {activeTab === "notifications" && (
                <SettingsNotificationsTab
                  notificationsEnabled={notificationsEnabled}
                  handleNotificationsToggle={handleNotificationsToggle}
                  smartNotifications={smartNotifications}
                  setSmartNotifications={setSmartNotifications}
                  notifyCategories={notifyCategories}
                  setNotifyCategories={setNotifyCategories}
                  vipSenders={vipSenders}
                  setVipSenders={setVipSenders}
                  newVipEmail={newVipEmail}
                  setNewVipEmail={setNewVipEmail}
                />
              )}

              {activeTab === "composing" && (
                <SettingsComposingTab
                  undoSendDelay={undoSendDelay}
                  handleUndoDelayChange={handleUndoDelayChange}
                />
              )}

              {activeTab === "mail-rules" && <SettingsMailRulesTab />}

              {activeTab === "people" && <SettingsPeopleTab />}

              {activeTab === "accounts" && (
                <SettingsAccountsTab
                  clientId={clientId}
                  setClientId={setClientId}
                  clientSecret={clientSecret}
                  setClientSecret={setClientSecret}
                  microsoftClientId={microsoftClientId}
                  setMicrosoftClientId={setMicrosoftClientId}
                  apiSettingsSaved={apiSettingsSaved}
                  handleSaveApiSettings={handleSaveApiSettings}
                  isSyncing={isSyncing}
                  handleManualSync={handleManualSync}
                  handleForceFullSync={handleForceFullSync}
                  syncPeriodDays={syncPeriodDays}
                  setSyncPeriodDays={setSyncPeriodDays}
                  handleRemoveAccount={handleRemoveAccount}
                  handleReauthorizeAccount={handleReauthorizeAccount}
                  handleResyncAccount={handleResyncAccount}
                  reauthStatus={reauthStatus}
                  resyncStatus={resyncStatus}
                />
              )}

              {activeTab === "shortcuts" && <SettingsShortcutsTab />}

              {activeTab === "ai" && (
                <SettingsAiTab
                  aiProvider={aiProvider}
                  setAiProvider={setAiProvider}
                  claudeApiKey={claudeApiKey}
                  setClaudeApiKey={setClaudeApiKey}
                  openaiApiKey={openaiApiKey}
                  setOpenaiApiKey={setOpenaiApiKey}
                  geminiApiKey={geminiApiKey}
                  setGeminiApiKey={setGeminiApiKey}
                  copilotApiKey={copilotApiKey}
                  setCopilotApiKey={setCopilotApiKey}
                  ollamaServerUrl={ollamaServerUrl}
                  setOllamaServerUrl={setOllamaServerUrl}
                  ollamaModel={ollamaModel}
                  setOllamaModel={setOllamaModel}
                  claudeModel={claudeModel}
                  setClaudeModel={setClaudeModel}
                  openaiModel={openaiModel}
                  setOpenaiModel={setOpenaiModel}
                  geminiModel={geminiModel}
                  setGeminiModel={setGeminiModel}
                  copilotModel={copilotModel}
                  setCopilotModel={setCopilotModel}
                  aiEnabled={aiEnabled}
                  setAiEnabled={setAiEnabled}
                  aiAutoCategorize={aiAutoCategorize}
                  setAiAutoCategorize={setAiAutoCategorize}
                  aiAutoSummarize={aiAutoSummarize}
                  setAiAutoSummarize={setAiAutoSummarize}
                  aiKeySaved={aiKeySaved}
                  setAiKeySaved={setAiKeySaved}
                  aiTesting={aiTesting}
                  setAiTesting={setAiTesting}
                  aiTestResult={aiTestResult}
                  setAiTestResult={setAiTestResult}
                  aiAutoDraftEnabled={aiAutoDraftEnabled}
                  setAiAutoDraftEnabled={setAiAutoDraftEnabled}
                  aiWritingStyleEnabled={aiWritingStyleEnabled}
                  setAiWritingStyleEnabled={setAiWritingStyleEnabled}
                  styleAnalyzing={styleAnalyzing}
                  setStyleAnalyzing={setStyleAnalyzing}
                  styleAnalyzeDone={styleAnalyzeDone}
                  setStyleAnalyzeDone={setStyleAnalyzeDone}
                  autoArchiveCategories={autoArchiveCategories}
                  setAutoArchiveCategories={setAutoArchiveCategories}
                />
              )}

              {activeTab === "about" && <SettingsAboutTab />}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
