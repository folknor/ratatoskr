import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { getStoredLanguagePreference } from "@/services/settings/runtimeFlags";
import accountsEN from "./locales/en/accounts.json";
import attachmentsEN from "./locales/en/attachments.json";
import calendarEN from "./locales/en/calendar.json";
import commonEN from "./locales/en/common.json";
import composerEN from "./locales/en/composer.json";
import emailEN from "./locales/en/email.json";
import helpEN from "./locales/en/help.json";
import notificationsEN from "./locales/en/notifications.json";
import searchEN from "./locales/en/search.json";
import settingsEN from "./locales/en/settings.json";
import sidebarEN from "./locales/en/sidebar.json";
import tasksEN from "./locales/en/tasks.json";

export const SUPPORTED_LANGUAGES = [{ code: "en", name: "English" }] as const;

export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number]["code"];

i18n.use(initReactI18next).init({
  resources: {
    en: {
      common: commonEN,
      sidebar: sidebarEN,
      email: emailEN,
      composer: composerEN,
      settings: settingsEN,
      search: searchEN,
      accounts: accountsEN,
      tasks: tasksEN,
      notifications: notificationsEN,
      calendar: calendarEN,
      attachments: attachmentsEN,
      help: helpEN,
    },
  },
  supportedLngs: ["en"],
  load: "languageOnly",
  fallbackLng: "en",
  defaultNS: "common",
  interpolation: {
    escapeValue: false,
  },
  lng: "en",
});

/**
 * Detect the OS system locale via Tauri plugin-os.
 * Falls back to navigator.language if Tauri is not available (e.g. in tests).
 */
async function detectSystemLanguage(): Promise<string> {
  try {
    const { locale } = await import("@tauri-apps/plugin-os");
    const osLocale = await locale();
    if (osLocale) {
      return osLocale.split("-")[0] ?? "en";
    }
  } catch {
    // Tauri not available (tests, plain browser)
  }
  return navigator.language?.split("-")[0] ?? "en";
}

/**
 * Change the app language and persist it to SQLite settings.
 */
export async function setAppLanguage(lang: SupportedLanguage): Promise<void> {
  await i18n.changeLanguage(lang);
  const { setSetting } = await import("@/services/db/settings");
  await setSetting("language", lang);
}

/**
 * Reset to system language (Tauri OS detection) and clear persisted preference.
 */
export async function resetToSystemLanguage(): Promise<void> {
  const { setSetting } = await import("@/services/db/settings");
  await setSetting("language", "system");
  const detected = await detectSystemLanguage();
  const supported = SUPPORTED_LANGUAGES.some((l) => l.code === detected);
  await i18n.changeLanguage(supported ? detected : "en");
}

/**
 * Load persisted language from SQLite. If not set or "system", detect from OS.
 * Call this early in app init, after migrations.
 */
export async function loadPersistedLanguage(): Promise<void> {
  const saved = await getStoredLanguagePreference();
  if (
    saved &&
    saved !== "system" &&
    SUPPORTED_LANGUAGES.some((l) => l.code === saved)
  ) {
    await i18n.changeLanguage(saved);
  } else {
    // No explicit preference — detect from OS
    const detected = await detectSystemLanguage();
    const supported = SUPPORTED_LANGUAGES.some((l) => l.code === detected);
    await i18n.changeLanguage(supported ? detected : "en");
  }
}

/**
 * Check if the user has explicitly chosen a language (vs using system default).
 */
export async function getPersistedLanguage(): Promise<string | null> {
  const saved = await getStoredLanguagePreference();
  if (
    saved &&
    saved !== "system" &&
    SUPPORTED_LANGUAGES.some((l) => l.code === saved)
  ) {
    return saved;
  }
  return null;
}

/**
 * Detect the system language and return its native display name.
 * Used for the language selector to show "Default (English)" / "Default (Italiano)".
 */
export async function getSystemLanguageName(): Promise<string> {
  const detected = await detectSystemLanguage();
  const lang = SUPPORTED_LANGUAGES.find((l) => l.code === detected);
  return lang?.name ?? "English";
}

export default i18n;
