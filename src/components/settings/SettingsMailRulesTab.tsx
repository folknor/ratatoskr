import type React from "react";
import { useTranslation } from "react-i18next";
import { FilterEditor } from "./FilterEditor";
import { LabelEditor } from "./LabelEditor";
import { QuickStepEditor } from "./QuickStepEditor";
import { Section } from "./SettingsShared";
import { SmartFolderEditor } from "./SmartFolderEditor";
import { SmartLabelEditor } from "./SmartLabelEditor";

export function SettingsMailRulesTab(): React.ReactNode {
  const { t } = useTranslation("settings");

  return (
    <>
      <Section title={t("labelsSection")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("labelsDescription")}
        </p>
        <LabelEditor />
      </Section>

      <Section title={t("filtersSection")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("filtersDescription")}
        </p>
        <FilterEditor />
      </Section>

      <Section title={t("smartLabels")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("smartLabelsDescription")}
        </p>
        <SmartLabelEditor />
      </Section>

      <Section title={t("smartFolders")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("smartFoldersDescription")}{" "}
          <code className="bg-bg-tertiary px-1 rounded">is:unread</code>,{" "}
          <code className="bg-bg-tertiary px-1 rounded">from:</code>,{" "}
          <code className="bg-bg-tertiary px-1 rounded">has:attachment</code>,{" "}
          <code className="bg-bg-tertiary px-1 rounded">after:</code>.
        </p>
        <SmartFolderEditor />
      </Section>

      <Section title={t("quickSteps")}>
        <p className="text-xs text-text-tertiary mb-3">
          {t("quickStepsDescription")}
        </p>
        <QuickStepEditor />
      </Section>
    </>
  );
}
