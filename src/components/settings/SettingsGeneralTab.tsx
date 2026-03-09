import { Check, ChevronDown, ChevronUp, RotateCcw } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import { ALL_NAV_ITEMS } from "@/components/layout/Sidebar";
import { Button } from "@/components/ui/Button";
import { COLOR_THEMES } from "@/constants/themes";
import {
  resetToSystemLanguage,
  SUPPORTED_LANGUAGES,
  setAppLanguage,
  type SupportedLanguage,
} from "@/i18n";
import { setSetting } from "@/core/settings";
import type { SidebarNavItem } from "@/stores/uiStore";
import { useUIStore } from "@/stores/uiStore";
import { Section, SettingRow, ToggleRow } from "./SettingsShared";

export interface SettingsGeneralTabProps {
  languageOverride: string | null;
  setLanguageOverride: (val: string | null) => void;
  languageLoaded: boolean;
  systemLanguageName: string;
  autostartEnabled: boolean;
  handleAutostartToggle: () => Promise<void>;
  blockRemoteImages: boolean;
  setBlockRemoteImages: (val: boolean) => void;
  phishingDetectionEnabled: boolean;
  setPhishingDetectionEnabled: (val: boolean) => void;
  phishingSensitivity: "low" | "default" | "high";
  setPhishingSensitivity: (val: "low" | "default" | "high") => void;
  cacheSizeMb: number | null;
  cacheMaxMb: string;
  setCacheMaxMb: (val: string) => void;
  clearingCache: boolean;
  setClearingCache: (val: boolean) => void;
  setCacheSizeMb: (val: number) => void;
}

export function SettingsGeneralTab({
  languageOverride,
  setLanguageOverride,
  languageLoaded,
  systemLanguageName,
  autostartEnabled,
  handleAutostartToggle,
  blockRemoteImages,
  setBlockRemoteImages,
  phishingDetectionEnabled,
  setPhishingDetectionEnabled,
  phishingSensitivity,
  setPhishingSensitivity,
  cacheSizeMb,
  cacheMaxMb,
  setCacheMaxMb,
  clearingCache,
  setClearingCache,
  setCacheSizeMb,
}: SettingsGeneralTabProps): React.ReactNode {
  const { t } = useTranslation("settings");
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const readingPanePosition = useUIStore((s) => s.readingPanePosition);
  const setReadingPanePosition = useUIStore((s) => s.setReadingPanePosition);
  const emailDensity = useUIStore((s) => s.emailDensity);
  const setEmailDensity = useUIStore((s) => s.setEmailDensity);
  const fontScale = useUIStore((s) => s.fontScale);
  const setFontScale = useUIStore((s) => s.setFontScale);
  const colorTheme = useUIStore((s) => s.colorTheme);
  const setColorTheme = useUIStore((s) => s.setColorTheme);
  const inboxViewMode = useUIStore((s) => s.inboxViewMode);
  const setInboxViewMode = useUIStore((s) => s.setInboxViewMode);
  const showSyncStatusBar = useUIStore((s) => s.showSyncStatusBar);
  const setShowSyncStatusBar = useUIStore((s) => s.setShowSyncStatusBar);
  const reduceMotion = useUIStore((s) => s.reduceMotion);
  const setReduceMotion = useUIStore((s) => s.setReduceMotion);

  return (
    <>
      <Section title={t("language")}>
        <SettingRow label={t("language")}>
          <select
            value={languageLoaded ? (languageOverride ?? "system") : "system"}
            onChange={async (
              e: React.ChangeEvent<HTMLSelectElement>,
            ): Promise<void> => {
              const val = e.target.value;
              if (val === "system") {
                setLanguageOverride(null);
                await resetToSystemLanguage();
              } else {
                const lang = val as SupportedLanguage;
                setLanguageOverride(lang);
                await setAppLanguage(lang);
              }
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="system">
              {t("languageDefaultWithName", {
                language: systemLanguageName,
              })}
            </option>
            {SUPPORTED_LANGUAGES.map((lang) => (
              <option key={lang.code} value={lang.code}>
                {lang.name}
              </option>
            ))}
          </select>
        </SettingRow>
      </Section>
      <Section title={t("appearance")}>
        <SettingRow label={t("theme")}>
          <select
            value={theme}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              const val = e.target.value as "light" | "dark" | "system";
              setTheme(val);
              setSetting("theme", val);
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="system">{t("themeSystem")}</option>
            <option value="light">{t("themeLight")}</option>
            <option value="dark">{t("themeDark")}</option>
          </select>
        </SettingRow>
        <SettingRow label={t("readingPane")}>
          <select
            value={readingPanePosition}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              setReadingPanePosition(
                e.target.value as "right" | "bottom" | "hidden",
              );
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="right">{t("readingPaneRight")}</option>
            <option value="bottom">{t("readingPaneBottom")}</option>
            <option value="hidden">{t("readingPaneOff")}</option>
          </select>
        </SettingRow>
        <SettingRow label={t("emailDensity")}>
          <select
            value={emailDensity}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              setEmailDensity(
                e.target.value as "compact" | "default" | "spacious",
              );
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="compact">{t("densityCompact")}</option>
            <option value="default">{t("densityDefault")}</option>
            <option value="spacious">{t("densitySpacious")}</option>
          </select>
        </SettingRow>
        <SettingRow label={t("fontSize")}>
          <select
            value={fontScale}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              setFontScale(
                e.target.value as "small" | "default" | "large" | "xlarge",
              );
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="small">{t("fontSmall")}</option>
            <option value="default">{t("fontDefault")}</option>
            <option value="large">{t("fontLarge")}</option>
            <option value="xlarge">{t("fontXLarge")}</option>
          </select>
        </SettingRow>
        <SettingRow label={t("accentColor")}>
          <div className="flex items-center gap-2">
            {COLOR_THEMES.map((ct) => {
              const isSelected = colorTheme === ct.id;
              return (
                <button
                  type="button"
                  key={ct.id}
                  onClick={(): void => setColorTheme(ct.id)}
                  title={ct.name}
                  className={`relative w-7 h-7 rounded-full transition-all ${
                    isSelected
                      ? "ring-2 ring-offset-2 ring-offset-bg-primary scale-110"
                      : "hover:scale-105"
                  }`}
                  style={{
                    backgroundColor: ct.swatch,
                    boxShadow: isSelected
                      ? `0 0 0 2px var(--color-bg-primary), 0 0 0 4px ${ct.swatch}`
                      : undefined,
                  }}
                >
                  {isSelected && (
                    <Check
                      size={14}
                      className="absolute inset-0 m-auto text-white drop-shadow-sm"
                    />
                  )}
                </button>
              );
            })}
          </div>
        </SettingRow>
        <SettingRow label={t("inboxViewMode")}>
          <select
            value={inboxViewMode}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              setInboxViewMode(e.target.value as "unified" | "split");
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="unified">{t("viewModeUnified")}</option>
            <option value="split">{t("viewModeSplit")}</option>
          </select>
        </SettingRow>
        <ToggleRow
          label="Show sync status bar"
          description="Display the syncing status bar at the bottom of the window"
          checked={showSyncStatusBar}
          onToggle={() => setShowSyncStatusBar(!showSyncStatusBar)}
        />
        <ToggleRow
          label={t("reduceMotion")}
          description={t("reduceMotionDescription")}
          checked={reduceMotion}
          onToggle={() => setReduceMotion(!reduceMotion)}
        />
      </Section>

      <SidebarNavEditor />

      <Section title={t("startup")}>
        <ToggleRow
          label={t("launchAtLogin")}
          description={t("launchAtLoginDescription")}
          checked={autostartEnabled}
          onToggle={handleAutostartToggle}
        />
      </Section>

      <Section title={t("privacySecurity")}>
        <ToggleRow
          label={t("blockRemoteImages")}
          description={t("blockRemoteImagesDescription")}
          checked={blockRemoteImages}
          onToggle={async () => {
            const newVal = !blockRemoteImages;
            setBlockRemoteImages(newVal);
            await setSetting("block_remote_images", newVal ? "true" : "false");
          }}
        />
        <ToggleRow
          label={t("phishingDetection")}
          description={t("phishingDescription")}
          checked={phishingDetectionEnabled}
          onToggle={async () => {
            const newVal = !phishingDetectionEnabled;
            setPhishingDetectionEnabled(newVal);
            await setSetting(
              "phishing_detection_enabled",
              newVal ? "true" : "false",
            );
          }}
        />
        {phishingDetectionEnabled === true && (
          <SettingRow label={t("detectionSensitivity")}>
            <select
              value={phishingSensitivity}
              onChange={async (
                e: React.ChangeEvent<HTMLSelectElement>,
              ): Promise<void> => {
                const val = e.target.value as "low" | "default" | "high";
                setPhishingSensitivity(val);
                await setSetting("phishing_sensitivity", val);
              }}
              className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
            >
              <option value="low">{t("sensitivityLow")}</option>
              <option value="default">{t("sensitivityDefault")}</option>
              <option value="high">{t("sensitivityHigh")}</option>
            </select>
          </SettingRow>
        )}
      </Section>

      <Section title={t("storage")}>
        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm text-text-secondary">
              {t("attachmentCache")}
            </span>
            <p className="text-xs text-text-tertiary mt-0.5">
              {cacheSizeMb !== null
                ? t("mbUsed", { size: cacheSizeMb })
                : t("calculating")}
            </p>
          </div>
          <Button
            variant="secondary"
            onClick={async () => {
              setClearingCache(true);
              try {
                const { clearAllCache } = await import(
                  "@/services/attachments/cacheManager"
                );
                await clearAllCache();
                setCacheSizeMb(0);
              } catch (err) {
                console.error("Failed to clear cache:", err);
              } finally {
                setClearingCache(false);
              }
            }}
            disabled={clearingCache}
            className="bg-bg-tertiary text-text-primary border border-border-primary"
          >
            {clearingCache ? t("clearing") : t("clearCache")}
          </Button>
        </div>
        <SettingRow label={t("maxCacheSize")}>
          <select
            value={cacheMaxMb}
            onChange={async (
              e: React.ChangeEvent<HTMLSelectElement>,
            ): Promise<void> => {
              const val = e.target.value;
              setCacheMaxMb(val);
              await setSetting("attachment_cache_max_mb", val);
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="100">{t("cache100mb")}</option>
            <option value="250">{t("cache250mb")}</option>
            <option value="500">{t("cache500mb")}</option>
            <option value="1000">{t("cache1gb")}</option>
            <option value="2000">{t("cache2gb")}</option>
          </select>
        </SettingRow>
      </Section>
    </>
  );
}

function SidebarNavEditor(): React.ReactNode {
  const { t } = useTranslation("settings");
  const sidebarNavConfig = useUIStore((s) => s.sidebarNavConfig);
  const setSidebarNavConfig = useUIStore((s) => s.setSidebarNavConfig);

  const items: SidebarNavItem[] = (() => {
    if (!sidebarNavConfig)
      return ALL_NAV_ITEMS.map((i) => ({ id: i.id, visible: true }));
    // Append any ALL_NAV_ITEMS entries missing from saved config (e.g. newly added sections)
    const savedIds = new Set(sidebarNavConfig.map((i) => i.id));
    const missing = ALL_NAV_ITEMS.filter((i) => !savedIds.has(i.id)).map(
      (i) => ({ id: i.id, visible: true }),
    );
    return [...sidebarNavConfig, ...missing];
  })();
  const navLookup = new Map(ALL_NAV_ITEMS.map((n) => [n.id, n]));

  const moveItem = (index: number, direction: -1 | 1): void => {
    const next = [...items];
    const target = index + direction;
    if (target < 0 || target >= next.length) return;
    const a = next[index];
    const b = next[target];
    if (!(a && b)) return;
    next[index] = b;
    next[target] = a;
    setSidebarNavConfig(next);
  };

  const toggleItem = (index: number): void => {
    const next = [...items];
    const current = next[index];
    // Inbox cannot be hidden
    if (!current || current.id === "inbox") return;
    next[index] = { ...current, visible: !current.visible };
    setSidebarNavConfig(next);
  };

  const resetToDefaults = (): void => {
    setSidebarNavConfig(
      ALL_NAV_ITEMS.map((i) => ({ id: i.id, visible: true })),
    );
  };

  const isDefault =
    !sidebarNavConfig ||
    (items.length === ALL_NAV_ITEMS.length &&
      items.every(
        (item, i) => item.id === ALL_NAV_ITEMS[i]?.id && item.visible,
      ));

  return (
    <Section title={t("sidebar")}>
      <div className="space-y-1">
        {items.map((item, index) => {
          const nav = navLookup.get(item.id);
          if (!nav) return null;
          const Icon = nav.icon;
          const isInbox = item.id === "inbox";
          return (
            <div
              key={item.id}
              className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors ${
                item.visible ? "text-text-primary" : "text-text-tertiary"
              }`}
            >
              <button
                type="button"
                onClick={(): void => moveItem(index, -1)}
                disabled={index === 0}
                className="p-0.5 rounded text-text-tertiary hover:text-text-primary disabled:opacity-25 disabled:cursor-not-allowed transition-colors"
                title={t("moveUp")}
              >
                <ChevronUp size={14} />
              </button>
              <button
                type="button"
                onClick={(): void => moveItem(index, 1)}
                disabled={index === items.length - 1}
                className="p-0.5 rounded text-text-tertiary hover:text-text-primary disabled:opacity-25 disabled:cursor-not-allowed transition-colors"
                title={t("moveDown")}
              >
                <ChevronDown size={14} />
              </button>
              <Icon size={16} className="shrink-0 ml-1" />
              <span className="flex-1 truncate">
                {t(
                  `sidebar:${item.id === "all" ? "allMail" : item.id === "smart-folders" ? "smartFolders" : item.id}`,
                )}
              </span>
              <button
                type="button"
                onClick={(): void => toggleItem(index)}
                disabled={isInbox}
                className={`relative w-10 h-5 rounded-full transition-colors shrink-0 ${
                  isInbox
                    ? "bg-accent/40 cursor-not-allowed"
                    : item.visible
                      ? "bg-accent cursor-pointer"
                      : "bg-bg-tertiary cursor-pointer"
                }`}
                title={
                  isInbox
                    ? t("inboxAlwaysVisible")
                    : item.visible
                      ? t("hide")
                      : t("show")
                }
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full shadow transition-transform ${
                    item.visible ? "translate-x-5" : ""
                  }`}
                />
              </button>
            </div>
          );
        })}
      </div>
      {!isDefault && (
        <button
          type="button"
          onClick={resetToDefaults}
          className="flex items-center gap-1.5 text-xs text-accent hover:text-accent-hover mt-2 transition-colors"
        >
          <RotateCcw size={12} />
          {t("resetToDefaults")}
        </button>
      )}
    </Section>
  );
}
