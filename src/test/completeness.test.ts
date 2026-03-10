import { describe, expect, it } from "vitest";
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
import accountsIT from "./locales/nb/accounts.json";
import attachmentsIT from "./locales/nb/attachments.json";
import calendarIT from "./locales/nb/calendar.json";
import commonIT from "./locales/nb/common.json";
import composerIT from "./locales/nb/composer.json";
import emailIT from "./locales/nb/email.json";
import helpIT from "./locales/nb/help.json";
import notificationsIT from "./locales/nb/notifications.json";
import searchIT from "./locales/nb/search.json";
import settingsIT from "./locales/nb/settings.json";
import sidebarIT from "./locales/nb/sidebar.json";
import tasksIT from "./locales/nb/tasks.json";

const namespaces: Record<
  string,
  { en: Record<string, unknown>; nb_NO: Record<string, unknown> }
> = {
  common: { en: commonEN, nb_NO: commonIT },
  sidebar: { en: sidebarEN, nb_NO: sidebarIT },
  email: { en: emailEN, nb_NO: emailIT },
  composer: { en: composerEN, nb_NO: composerIT },
  settings: { en: settingsEN, nb_NO: settingsIT },
  search: { en: searchEN, nb_NO: searchIT },
  accounts: { en: accountsEN, nb_NO: accountsIT },
  tasks: { en: tasksEN, nb_NO: tasksIT },
  notifications: { en: notificationsEN, nb_NO: notificationsIT },
  calendar: { en: calendarEN, nb_NO: calendarIT },
  attachments: { en: attachmentsEN, nb_NO: attachmentsIT },
  help: { en: helpEN, nb_NO: helpIT },
};

function getKeys(obj: Record<string, unknown>, prefix = ""): string[] {
  const keys: string[] = [];
  for (const key of Object.keys(obj)) {
    const fullKey = prefix ? `${prefix}.${key}` : key;
    const value = obj[key];
    if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      keys.push(...getKeys(value as Record<string, unknown>, fullKey));
    } else {
      keys.push(fullKey);
    }
  }
  return keys.sort((a, b) => a.localeCompare(b));
}

describe("Translation completeness", () => {
  for (const [ns, { en, nb_NO }] of Object.entries(namespaces)) {
    describe(`namespace: ${ns}`, () => {
      it("nb_NO should have all keys from EN", () => {
        const enKeys = getKeys(en);
        const itKeys = getKeys(nb_NO);
        const missing = enKeys.filter((k) => !itKeys.includes(k));
        expect(missing, `nb_NO missing keys: ${missing.join(", ")}`).toEqual(
          [],
        );
      });

      it("EN should have all keys from IT (no extra keys in IT)", () => {
        const enKeys = getKeys(en);
        const itKeys = getKeys(nb_NO);
        const extra = itKeys.filter((k) => !enKeys.includes(k));
        expect(extra, `nb_NO has extra keys: ${extra.join(", ")}`).toEqual([]);
      });

      it("should not have empty string values in EN", () => {
        const enKeys = getKeys(en);
        const empty = enKeys.filter((k) => {
          const parts = k.split(".");
          let val: unknown = en;
          for (const p of parts) val = (val as Record<string, unknown>)[p];
          return val === "";
        });
        expect(empty, `EN has empty values: ${empty.join(", ")}`).toEqual([]);
      });

      it("should not have empty string values in IT", () => {
        const itKeys = getKeys(nb_NO);
        const empty = itKeys.filter((k) => {
          const parts = k.split(".");
          let val: unknown = nb_NO;
          for (const p of parts) val = (val as Record<string, unknown>)[p];
          return val === "";
        });
        expect(empty, `nb_NO has empty values: ${empty.join(", ")}`).toEqual(
          [],
        );
      });
    });
  }
});
