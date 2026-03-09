import type React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";
import { setSetting } from "@/core/settings";
import { useAccountStore } from "@/stores/accountStore";
import { Section, ToggleRow } from "./SettingsShared";

export interface SettingsNotificationsTabProps {
  notificationsEnabled: boolean;
  handleNotificationsToggle: () => Promise<void>;
  smartNotifications: boolean;
  setSmartNotifications: (val: boolean) => void;
  notifyCategories: Set<string>;
  setNotifyCategories: (val: Set<string>) => void;
  vipSenders: { email_address: string; display_name: string | null }[];
  setVipSenders: React.Dispatch<
    React.SetStateAction<
      { email_address: string; display_name: string | null }[]
    >
  >;
  newVipEmail: string;
  setNewVipEmail: (val: string) => void;
}

export function SettingsNotificationsTab({
  notificationsEnabled,
  handleNotificationsToggle,
  smartNotifications,
  setSmartNotifications,
  notifyCategories,
  setNotifyCategories,
  vipSenders,
  setVipSenders,
  newVipEmail,
  setNewVipEmail,
}: SettingsNotificationsTabProps): React.ReactNode {
  const { t } = useTranslation("settings");
  const accounts = useAccountStore((s) => s.accounts);

  return (
    <>
      <Section title={t("tabNotifications")}>
        <ToggleRow
          label={t("enableNotifications")}
          checked={notificationsEnabled}
          onToggle={handleNotificationsToggle}
        />
        <ToggleRow
          label={t("smartNotifications")}
          description={t("smartNotificationsDescription")}
          checked={smartNotifications}
          onToggle={async () => {
            const newVal = !smartNotifications;
            setSmartNotifications(newVal);
            await setSetting("smart_notifications", newVal ? "true" : "false");
          }}
        />
      </Section>

      {smartNotifications === true && (
        <>
          <Section title={t("notifyForCategories")}>
            <div>
              <span className="text-sm text-text-secondary">
                {t("notifyForCategories")}
              </span>
              <div className="flex flex-wrap gap-2 mt-2">
                {(
                  [
                    "Primary",
                    "Updates",
                    "Promotions",
                    "Social",
                    "Newsletters",
                  ] as const
                ).map((cat) => (
                  <button
                    type="button"
                    key={cat}
                    onClick={async (): Promise<void> => {
                      const next = new Set(notifyCategories);
                      if (next.has(cat)) next.delete(cat);
                      else next.add(cat);
                      setNotifyCategories(next);
                      await setSetting(
                        "notify_categories",
                        [...next].join(","),
                      );
                    }}
                    className={`px-2.5 py-1 text-xs rounded-full transition-colors border ${
                      notifyCategories.has(cat)
                        ? "bg-accent/15 text-accent border-accent/30"
                        : "bg-bg-tertiary text-text-tertiary border-border-primary hover:text-text-primary"
                    }`}
                  >
                    {t(`sidebar:${cat.toLowerCase()}`)}
                  </button>
                ))}
              </div>
            </div>
          </Section>

          <Section title={t("vipSenders")}>
            <p className="text-xs text-text-tertiary mb-2">
              {t("vipDescription")}
            </p>
            <div className="space-y-1.5">
              {vipSenders.map((vip) => (
                <div
                  key={vip.email_address}
                  className="flex items-center justify-between py-1.5 px-3 bg-bg-secondary rounded-md"
                >
                  <span className="text-xs text-text-primary truncate">
                    {vip.display_name
                      ? `${vip.display_name} (${vip.email_address})`
                      : vip.email_address}
                  </span>
                  <button
                    type="button"
                    onClick={async (): Promise<void> => {
                      const activeId = accounts.find((a) => a.isActive)?.id;
                      if (!activeId) return;
                      const { removeVipSender } = await import(
                        "@/services/db/notificationVips"
                      );
                      await removeVipSender(activeId, vip.email_address);
                      setVipSenders((prev) =>
                        prev.filter(
                          (v) => v.email_address !== vip.email_address,
                        ),
                      );
                    }}
                    className="text-xs text-danger hover:text-danger/80 ml-2 shrink-0"
                  >
                    {t("remove")}
                  </button>
                </div>
              ))}
            </div>
            <div className="flex gap-2 mt-2">
              <input
                type="email"
                value={newVipEmail}
                onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                  setNewVipEmail(e.target.value)
                }
                placeholder={t("vipPlaceholder")}
                className="flex-1 px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded-md text-xs text-text-primary outline-none focus:border-accent"
                onKeyDown={async (
                  e: React.KeyboardEvent<HTMLInputElement>,
                ): Promise<void> => {
                  if (e.key !== "Enter" || !newVipEmail.trim()) return;
                  const activeId = accounts.find((a) => a.isActive)?.id;
                  if (!activeId) return;
                  const { addVipSender } = await import(
                    "@/services/db/notificationVips"
                  );
                  await addVipSender(activeId, newVipEmail.trim());
                  setVipSenders((prev) => [
                    ...prev,
                    {
                      email_address: newVipEmail.trim().toLowerCase(),
                      display_name: null,
                    },
                  ]);
                  setNewVipEmail("");
                }}
              />
              <Button
                variant="primary"
                onClick={async () => {
                  if (!newVipEmail.trim()) return;
                  const activeId = accounts.find((a) => a.isActive)?.id;
                  if (!activeId) return;
                  const { addVipSender } = await import(
                    "@/services/db/notificationVips"
                  );
                  await addVipSender(activeId, newVipEmail.trim());
                  setVipSenders((prev) => [
                    ...prev,
                    {
                      email_address: newVipEmail.trim().toLowerCase(),
                      display_name: null,
                    },
                  ]);
                  setNewVipEmail("");
                }}
                disabled={!newVipEmail.trim()}
              >
                {t("add")}
              </Button>
            </div>
          </Section>
        </>
      )}
    </>
  );
}
