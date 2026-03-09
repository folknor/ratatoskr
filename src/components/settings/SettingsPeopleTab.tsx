import type React from "react";
import { useTranslation } from "react-i18next";
import { ContactEditor } from "./ContactEditor";
import { Section } from "./SettingsShared";
import { SubscriptionManager } from "./SubscriptionManager";

export function SettingsPeopleTab(): React.ReactNode {
  const { t } = useTranslation("settings");

  return (
    <>
      <Section title={t("contacts")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("contactsDescription")}
        </p>
        <ContactEditor />
      </Section>

      <Section title={t("subscriptions")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("subscriptionsDescription")}
        </p>
        <SubscriptionManager />
      </Section>
    </>
  );
}
