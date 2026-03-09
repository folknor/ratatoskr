import type React from "react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";
import { TextField } from "@/components/ui/TextField";
import { PROVIDER_MODELS } from "@/services/ai/types";
import { setSecureSetting, setSetting } from "@/services/db/settings";
import { useAccountStore } from "@/stores/accountStore";
import { Section, SettingRow, ToggleRow } from "./SettingsShared";

export interface SettingsAiTabProps {
  aiProvider: "claude" | "openai" | "gemini" | "ollama" | "copilot";
  setAiProvider: (
    val: "claude" | "openai" | "gemini" | "ollama" | "copilot",
  ) => void;
  claudeApiKey: string;
  setClaudeApiKey: (val: string) => void;
  openaiApiKey: string;
  setOpenaiApiKey: (val: string) => void;
  geminiApiKey: string;
  setGeminiApiKey: (val: string) => void;
  copilotApiKey: string;
  setCopilotApiKey: (val: string) => void;
  ollamaServerUrl: string;
  setOllamaServerUrl: (val: string) => void;
  ollamaModel: string;
  setOllamaModel: (val: string) => void;
  claudeModel: string;
  setClaudeModel: (val: string) => void;
  openaiModel: string;
  setOpenaiModel: (val: string) => void;
  geminiModel: string;
  setGeminiModel: (val: string) => void;
  copilotModel: string;
  setCopilotModel: (val: string) => void;
  aiEnabled: boolean;
  setAiEnabled: (val: boolean) => void;
  aiAutoCategorize: boolean;
  setAiAutoCategorize: (val: boolean) => void;
  aiAutoSummarize: boolean;
  setAiAutoSummarize: (val: boolean) => void;
  aiKeySaved: boolean;
  setAiKeySaved: (val: boolean) => void;
  aiTesting: boolean;
  setAiTesting: (val: boolean) => void;
  aiTestResult: "success" | "fail" | null;
  setAiTestResult: (val: "success" | "fail" | null) => void;
  aiAutoDraftEnabled: boolean;
  setAiAutoDraftEnabled: (val: boolean) => void;
  aiWritingStyleEnabled: boolean;
  setAiWritingStyleEnabled: (val: boolean) => void;
  styleAnalyzing: boolean;
  setStyleAnalyzing: (val: boolean) => void;
  styleAnalyzeDone: boolean;
  setStyleAnalyzeDone: (val: boolean) => void;
  autoArchiveCategories: Set<string>;
  setAutoArchiveCategories: (val: Set<string>) => void;
}

export function SettingsAiTab({
  aiProvider,
  setAiProvider,
  claudeApiKey,
  setClaudeApiKey,
  openaiApiKey,
  setOpenaiApiKey,
  geminiApiKey,
  setGeminiApiKey,
  copilotApiKey,
  setCopilotApiKey,
  ollamaServerUrl,
  setOllamaServerUrl,
  ollamaModel,
  setOllamaModel,
  claudeModel,
  setClaudeModel,
  openaiModel,
  setOpenaiModel,
  geminiModel,
  setGeminiModel,
  copilotModel,
  setCopilotModel,
  aiEnabled,
  setAiEnabled,
  aiAutoCategorize,
  setAiAutoCategorize,
  aiAutoSummarize,
  setAiAutoSummarize,
  aiKeySaved,
  setAiKeySaved,
  aiTesting,
  setAiTesting,
  aiTestResult,
  setAiTestResult,
  aiAutoDraftEnabled,
  setAiAutoDraftEnabled,
  aiWritingStyleEnabled,
  setAiWritingStyleEnabled,
  styleAnalyzing,
  setStyleAnalyzing,
  styleAnalyzeDone,
  setStyleAnalyzeDone,
  autoArchiveCategories,
  setAutoArchiveCategories,
}: SettingsAiTabProps): React.ReactNode {
  const { t } = useTranslation("settings");
  const accounts = useAccountStore((s) => s.accounts);

  return (
    <>
      <Section title={t("provider")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("providerDescription")}
        </p>
        <SettingRow label={t("aiProvider")}>
          <select
            value={aiProvider}
            onChange={async (
              e: React.ChangeEvent<HTMLSelectElement>,
            ): Promise<void> => {
              const val = e.target.value as
                | "claude"
                | "openai"
                | "gemini"
                | "ollama"
                | "copilot";
              setAiProvider(val);
              setAiTestResult(null);
              await setSetting("ai_provider", val);
              const { clearProviderClients } = await import(
                "@/services/ai/providerManager"
              );
              clearProviderClients();
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="claude">{t("providerClaude")}</option>
            <option value="openai">{t("providerOpenAI")}</option>
            <option value="gemini">{t("providerGemini")}</option>
            <option value="ollama">{t("providerLocal")}</option>
            <option value="copilot">{t("providerCopilot")}</option>
          </select>
        </SettingRow>
        <p className="text-xs text-text-tertiary">
          {aiProvider === "claude" &&
            `${t("uses")} ${PROVIDER_MODELS.claude.find((m) => m.id === claudeModel)?.label ?? claudeModel}.`}
          {aiProvider === "openai" &&
            `${t("uses")} ${PROVIDER_MODELS.openai.find((m) => m.id === openaiModel)?.label ?? openaiModel}.`}
          {aiProvider === "gemini" &&
            `${t("uses")} ${PROVIDER_MODELS.gemini.find((m) => m.id === geminiModel)?.label ?? geminiModel}.`}
          {aiProvider === "ollama" && t("localAiDescription")}
          {aiProvider === "copilot" &&
            `${t("uses")} ${PROVIDER_MODELS.copilot.find((m) => m.id === copilotModel)?.label ?? copilotModel}. ${t("copilotDescription")}`}
        </p>
      </Section>

      {aiProvider === "ollama" ? (
        <Section title={t("localServer")}>
          <div className="space-y-3">
            <TextField
              label={t("serverUrl")}
              size="md"
              value={ollamaServerUrl}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                setOllamaServerUrl(e.target.value)
              }
              placeholder={t("localServerPlaceholder")}
            />
            <TextField
              label={t("modelName")}
              size="md"
              value={ollamaModel}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                setOllamaModel(e.target.value)
              }
              placeholder={t("modelPlaceholder")}
            />
            <div className="flex items-center gap-2">
              <Button
                variant="primary"
                size="md"
                onClick={async () => {
                  await setSetting("ollama_server_url", ollamaServerUrl.trim());
                  await setSetting("ollama_model", ollamaModel.trim());
                  const { clearProviderClients } = await import(
                    "@/services/ai/providerManager"
                  );
                  clearProviderClients();
                  setAiKeySaved(true);
                  setTimeout(() => setAiKeySaved(false), 2000);
                }}
                disabled={!(ollamaServerUrl.trim() && ollamaModel.trim())}
              >
                {aiKeySaved ? t("saved") : t("save")}
              </Button>
              <Button
                variant="secondary"
                size="md"
                onClick={async () => {
                  setAiTesting(true);
                  setAiTestResult(null);
                  try {
                    const { testConnection } = await import(
                      "@/services/ai/aiService"
                    );
                    const ok = await testConnection();
                    setAiTestResult(ok ? "success" : "fail");
                  } catch {
                    setAiTestResult("fail");
                  } finally {
                    setAiTesting(false);
                  }
                }}
                disabled={
                  !(ollamaServerUrl.trim() && ollamaModel.trim()) || aiTesting
                }
                className="bg-bg-tertiary text-text-primary border border-border-primary"
              >
                {aiTesting ? t("testing") : t("testConnection")}
              </Button>
              {aiTestResult === "success" && (
                <span className="text-xs text-success">{t("connected")}</span>
              )}
              {aiTestResult === "fail" && (
                <span className="text-xs text-danger">
                  {t("connectionFailed")}
                </span>
              )}
            </div>
          </div>
        </Section>
      ) : (
        <Section title={t("apiKey")}>
          <div className="space-y-3">
            <TextField
              label={
                aiProvider === "claude"
                  ? t("anthropicApiKey")
                  : aiProvider === "openai"
                    ? t("openaiApiKey")
                    : aiProvider === "copilot"
                      ? t("githubPat")
                      : t("googleAiApiKey")
              }
              size="md"
              type="password"
              value={
                aiProvider === "claude"
                  ? claudeApiKey
                  : aiProvider === "openai"
                    ? openaiApiKey
                    : aiProvider === "copilot"
                      ? copilotApiKey
                      : geminiApiKey
              }
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void => {
                if (aiProvider === "claude") setClaudeApiKey(e.target.value);
                else if (aiProvider === "openai")
                  setOpenaiApiKey(e.target.value);
                else if (aiProvider === "copilot")
                  setCopilotApiKey(e.target.value);
                else setGeminiApiKey(e.target.value);
              }}
              placeholder={
                aiProvider === "claude"
                  ? "sk-ant-..."
                  : aiProvider === "openai"
                    ? "sk-..."
                    : aiProvider === "copilot"
                      ? "ghp_..."
                      : "AI..."
              }
            />
            <SettingRow label={t("model")}>
              <select
                value={
                  aiProvider === "claude"
                    ? claudeModel
                    : aiProvider === "openai"
                      ? openaiModel
                      : aiProvider === "copilot"
                        ? copilotModel
                        : geminiModel
                }
                onChange={async (
                  e: React.ChangeEvent<HTMLSelectElement>,
                ): Promise<void> => {
                  const val = e.target.value;
                  const modelSettingMap = {
                    claude: "claude_model",
                    openai: "openai_model",
                    gemini: "gemini_model",
                    copilot: "copilot_model",
                  } as const;
                  if (aiProvider === "claude") setClaudeModel(val);
                  else if (aiProvider === "openai") setOpenaiModel(val);
                  else if (aiProvider === "copilot") setCopilotModel(val);
                  else setGeminiModel(val);
                  await setSetting(modelSettingMap[aiProvider], val);
                  const { clearProviderClients } = await import(
                    "@/services/ai/providerManager"
                  );
                  clearProviderClients();
                }}
                className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
              >
                {PROVIDER_MODELS[aiProvider].map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.label}
                  </option>
                ))}
              </select>
            </SettingRow>
            <div className="flex items-center gap-2">
              <Button
                variant="primary"
                size="md"
                onClick={async () => {
                  const keySettingMap = {
                    claude: "claude_api_key",
                    openai: "openai_api_key",
                    gemini: "gemini_api_key",
                    copilot: "copilot_api_key",
                  } as const;
                  const keyValue =
                    aiProvider === "claude"
                      ? claudeApiKey.trim()
                      : aiProvider === "openai"
                        ? openaiApiKey.trim()
                        : aiProvider === "copilot"
                          ? copilotApiKey.trim()
                          : geminiApiKey.trim();
                  if (keyValue) {
                    await setSecureSetting(keySettingMap[aiProvider], keyValue);
                    const { clearProviderClients } = await import(
                      "@/services/ai/providerManager"
                    );
                    clearProviderClients();
                  }
                  setAiKeySaved(true);
                  setTimeout(() => setAiKeySaved(false), 2000);
                }}
                disabled={
                  !(aiProvider === "claude"
                    ? claudeApiKey.trim()
                    : aiProvider === "openai"
                      ? openaiApiKey.trim()
                      : aiProvider === "copilot"
                        ? copilotApiKey.trim()
                        : geminiApiKey.trim())
                }
              >
                {aiKeySaved ? t("saved") : t("saveKey")}
              </Button>
              <Button
                variant="secondary"
                size="md"
                onClick={async () => {
                  setAiTesting(true);
                  setAiTestResult(null);
                  try {
                    const { testConnection } = await import(
                      "@/services/ai/aiService"
                    );
                    const ok = await testConnection();
                    setAiTestResult(ok ? "success" : "fail");
                  } catch {
                    setAiTestResult("fail");
                  } finally {
                    setAiTesting(false);
                  }
                }}
                disabled={
                  !(aiProvider === "claude"
                    ? claudeApiKey.trim()
                    : aiProvider === "openai"
                      ? openaiApiKey.trim()
                      : aiProvider === "copilot"
                        ? copilotApiKey.trim()
                        : geminiApiKey.trim()) || aiTesting
                }
                className="bg-bg-tertiary text-text-primary border border-border-primary"
              >
                {aiTesting ? t("testing") : t("testConnection")}
              </Button>
              {aiTestResult === "success" && (
                <span className="text-xs text-success">{t("connected")}</span>
              )}
              {aiTestResult === "fail" && (
                <span className="text-xs text-danger">
                  {t("connectionFailed")}
                </span>
              )}
            </div>
          </div>
        </Section>
      )}

      <Section title={t("features")}>
        <ToggleRow
          label={t("enableAiFeatures")}
          description={t("enableAiDescription")}
          checked={aiEnabled}
          onToggle={async () => {
            const newVal = !aiEnabled;
            setAiEnabled(newVal);
            await setSetting("ai_enabled", newVal ? "true" : "false");
          }}
        />
        <ToggleRow
          label={t("autoCategorize")}
          description={t("autoCategorizeDescription")}
          checked={aiAutoCategorize}
          onToggle={async () => {
            const newVal = !aiAutoCategorize;
            setAiAutoCategorize(newVal);
            await setSetting("ai_auto_categorize", newVal ? "true" : "false");
          }}
        />
        <ToggleRow
          label={t("autoSummarize")}
          description={t("autoSummarizeDescription")}
          checked={aiAutoSummarize}
          onToggle={async () => {
            const newVal = !aiAutoSummarize;
            setAiAutoSummarize(newVal);
            await setSetting("ai_auto_summarize", newVal ? "true" : "false");
          }}
        />
      </Section>

      <Section title={t("autoDraftReplies")}>
        <ToggleRow
          label={t("autoDraft")}
          description={t("autoDraftDescription")}
          checked={aiAutoDraftEnabled}
          onToggle={async () => {
            const newVal = !aiAutoDraftEnabled;
            setAiAutoDraftEnabled(newVal);
            await setSetting(
              "ai_auto_draft_enabled",
              newVal ? "true" : "false",
            );
          }}
        />
        <ToggleRow
          label={t("learnWritingStyle")}
          description={t("learnWritingStyleDescription")}
          checked={aiWritingStyleEnabled}
          onToggle={async () => {
            const newVal = !aiWritingStyleEnabled;
            setAiWritingStyleEnabled(newVal);
            await setSetting(
              "ai_writing_style_enabled",
              newVal ? "true" : "false",
            );
          }}
        />
        {aiWritingStyleEnabled === true && (
          <div className="flex items-center justify-between">
            <div>
              <span className="text-sm text-text-secondary">
                {t("writingStyleProfile")}
              </span>
              <p className="text-xs text-text-tertiary mt-0.5">
                {t("writingStyleReanalyze")}
              </p>
            </div>
            <Button
              variant="secondary"
              size="md"
              onClick={async () => {
                setStyleAnalyzing(true);
                setStyleAnalyzeDone(false);
                try {
                  const activeId = accounts.find((a) => a.isActive)?.id;
                  if (activeId) {
                    const { refreshWritingStyle } = await import(
                      "@/services/ai/writingStyleService"
                    );
                    await refreshWritingStyle(activeId);
                    setStyleAnalyzeDone(true);
                    setTimeout(() => setStyleAnalyzeDone(false), 3000);
                  }
                } catch (err) {
                  console.error("Style analysis failed:", err);
                } finally {
                  setStyleAnalyzing(false);
                }
              }}
              disabled={styleAnalyzing}
              className="bg-bg-tertiary text-text-primary border border-border-primary"
            >
              {styleAnalyzing
                ? t("analyzing")
                : styleAnalyzeDone
                  ? t("done")
                  : t("reanalyze")}
            </Button>
          </div>
        )}
      </Section>

      <Section title={t("categories")}>
        <p className="text-xs text-text-tertiary mb-1">
          {t("categoriesDescription")}
        </p>
        <p className="text-xs text-text-tertiary mb-3">
          {t("categoriesArchiveNote")}
        </p>
        {(["Updates", "Promotions", "Social", "Newsletters"] as const).map(
          (cat) => {
            const labelKey = `autoArchive${cat}` as const;
            const descKey = `autoArchive${cat}Desc` as const;
            return (
              <ToggleRow
                key={cat}
                label={t(labelKey)}
                description={t(descKey)}
                checked={autoArchiveCategories.has(cat)}
                onToggle={async () => {
                  const next = new Set(autoArchiveCategories);
                  if (next.has(cat)) next.delete(cat);
                  else next.add(cat);
                  setAutoArchiveCategories(next);
                  await setSetting(
                    "auto_archive_categories",
                    [...next].join(","),
                  );
                }}
              />
            );
          },
        )}
      </Section>

      <Section title={t("bundling")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("bundlingDescription")}
        </p>
        <BundleSettings />
      </Section>
    </>
  );
}

function BundleSettings(): React.ReactNode {
  const { t } = useTranslation("settings");
  const DAY_NAMES = [
    t("sun"),
    t("mon"),
    t("tue"),
    t("wed"),
    t("thu"),
    t("fri"),
    t("sat"),
  ];
  const accounts = useAccountStore((s) => s.accounts);
  const activeAccountId = accounts.find((a) => a.isActive)?.id;
  const [rules, setRules] = useState<
    Record<
      string,
      {
        bundled: boolean;
        delivery: boolean;
        days: number[];
        hour: number;
        minute: number;
      }
    >
  >({});

  useEffect(() => {
    if (!activeAccountId) return;
    void import("@/services/db/bundleRules").then(
      async ({ getBundleRules }) => {
        const dbRules = await getBundleRules(activeAccountId);
        const map: typeof rules = {};
        for (const r of dbRules) {
          let schedule = { days: [6], hour: 9, minute: 0 };
          try {
            if (r.delivery_schedule) schedule = JSON.parse(r.delivery_schedule);
          } catch {
            /* use defaults */
          }
          map[r.category] = {
            bundled: r.is_bundled === 1,
            delivery: r.delivery_enabled === 1,
            days: schedule.days,
            hour: schedule.hour,
            minute: schedule.minute,
          };
        }
        setRules(map);
      },
    );
  }, [activeAccountId]);

  const saveRule = async (
    category: string,
    update: Partial<(typeof rules)[string]>,
  ): Promise<void> => {
    if (!activeAccountId) return;
    const current = rules[category] ?? {
      bundled: false,
      delivery: false,
      days: [6],
      hour: 9,
      minute: 0,
    };
    const merged = { ...current, ...update };
    setRules((prev) => ({ ...prev, [category]: merged }));
    const { setBundleRule } = await import("@/services/db/bundleRules");
    await setBundleRule(
      activeAccountId,
      category,
      merged.bundled,
      merged.delivery,
      merged.delivery
        ? { days: merged.days, hour: merged.hour, minute: merged.minute }
        : null,
    );
  };

  return (
    <div className="space-y-4">
      {(["Newsletters", "Promotions", "Social", "Updates"] as const).map(
        (cat) => {
          const rule = rules[cat];
          return (
            <div
              key={cat}
              className="py-3 px-4 bg-bg-secondary rounded-lg space-y-2"
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-text-primary">
                  {t(`sidebar:${cat.toLowerCase()}`)}
                </span>
                <div className="flex items-center gap-3">
                  <label className="flex items-center gap-1.5 text-xs text-text-secondary">
                    <input
                      type="checkbox"
                      checked={rule?.bundled ?? false}
                      onChange={(): void =>
                        void saveRule(cat, {
                          bundled: !(rule?.bundled ?? false),
                        })
                      }
                      className="accent-accent"
                    />
                    {t("bundle")}
                  </label>
                  <label className="flex items-center gap-1.5 text-xs text-text-secondary">
                    <input
                      type="checkbox"
                      checked={rule?.delivery ?? false}
                      onChange={(): void =>
                        void saveRule(cat, {
                          delivery: !(rule?.delivery ?? false),
                        })
                      }
                      className="accent-accent"
                    />
                    {t("scheduleLabel")}
                  </label>
                </div>
              </div>
              {rule?.delivery === true && (
                <div className="space-y-2 pt-1">
                  <div className="flex gap-1">
                    {DAY_NAMES.map((name, idx) => (
                      <button
                        type="button"
                        key={name}
                        onClick={(): void => {
                          const days = rule.days.includes(idx)
                            ? rule.days.filter(
                                (d: number): boolean => d !== idx,
                              )
                            : [...rule.days, idx].sort();
                          void saveRule(cat, { days });
                        }}
                        className={`w-8 h-7 text-[0.625rem] rounded transition-colors ${
                          rule.days.includes(idx)
                            ? "bg-accent text-white"
                            : "bg-bg-tertiary text-text-tertiary border border-border-primary"
                        }`}
                      >
                        {name}
                      </button>
                    ))}
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-text-tertiary">
                      {t("at")}
                    </span>
                    <input
                      type="time"
                      value={`${String(rule.hour).padStart(2, "0")}:${String(rule.minute).padStart(2, "0")}`}
                      onChange={(
                        e: React.ChangeEvent<HTMLInputElement>,
                      ): void => {
                        const [h, m] = e.target.value.split(":").map(Number);
                        void saveRule(cat, { hour: h ?? 9, minute: m ?? 0 });
                      }}
                      className="bg-bg-tertiary text-text-primary text-xs px-2 py-1 rounded border border-border-primary"
                    />
                  </div>
                </div>
              )}
            </div>
          );
        },
      )}
    </div>
  );
}
