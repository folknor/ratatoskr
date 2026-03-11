import type React from "react";
import { useTranslation } from "react-i18next";
import { useUIPreferencesStore } from "@/stores/uiPreferencesStore";
import { Section, SettingRow, ToggleRow } from "./SettingsShared";
import { SignatureEditor } from "./SignatureEditor";
import { TemplateEditor } from "./TemplateEditor";

export interface SettingsComposingTabProps {
  undoSendDelay: string;
  handleUndoDelayChange: (value: string) => Promise<void>;
}

export function SettingsComposingTab({
  undoSendDelay,
  handleUndoDelayChange,
}: SettingsComposingTabProps): React.ReactNode {
  const { t } = useTranslation("settings");
  const defaultReplyMode = useUIPreferencesStore((s) => s.defaultReplyMode);
  const setDefaultReplyMode = useUIPreferencesStore(
    (s) => s.setDefaultReplyMode,
  );
  const markAsReadBehavior = useUIPreferencesStore((s) => s.markAsReadBehavior);
  const setMarkAsReadBehavior = useUIPreferencesStore(
    (s) => s.setMarkAsReadBehavior,
  );
  const sendAndArchive = useUIPreferencesStore((s) => s.sendAndArchive);
  const setSendAndArchive = useUIPreferencesStore((s) => s.setSendAndArchive);

  return (
    <>
      <Section title={t("sending")}>
        <SettingRow label={t("undoSendDelay")}>
          <select
            value={undoSendDelay}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              void handleUndoDelayChange(e.target.value);
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="0">{t("delayNone")}</option>
            <option value="5">{t("delay5s")}</option>
            <option value="10">{t("delay10s")}</option>
            <option value="30">{t("delay30s")}</option>
          </select>
        </SettingRow>
        <ToggleRow
          label={t("sendAndArchive")}
          description={t("sendAndArchiveDescription")}
          checked={sendAndArchive}
          onToggle={() => setSendAndArchive(!sendAndArchive)}
        />
      </Section>

      <Section title={t("behavior")}>
        <SettingRow label={t("defaultReplyAction")}>
          <select
            value={defaultReplyMode}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              setDefaultReplyMode(e.target.value as "reply" | "replyAll");
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="reply">{t("reply")}</option>
            <option value="replyAll">{t("replyAll")}</option>
          </select>
        </SettingRow>
        <SettingRow label={t("markAsRead")}>
          <select
            value={markAsReadBehavior}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void => {
              setMarkAsReadBehavior(
                e.target.value as "instant" | "2s" | "manual",
              );
            }}
            className="w-48 bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent outline-none"
          >
            <option value="instant">{t("markReadInstantly")}</option>
            <option value="2s">{t("markReadAfter2s")}</option>
            <option value="manual">{t("markReadManually")}</option>
          </select>
        </SettingRow>
      </Section>

      <Section title={t("signatures")}>
        <SignatureEditor />
      </Section>

      <Section title={t("templates")}>
        <TemplateEditor />
      </Section>
    </>
  );
}
