import { useTranslation } from "react-i18next";
import { useSelectedThreadId } from "@/hooks/useRouteNavigation";
import { useThreadStore } from "@/stores/threadStore";
import { ThreadView } from "../email/ThreadView";
import { EmptyState } from "../ui/EmptyState";
import { ReadingPaneIllustration } from "../ui/illustrations";

export function ReadingPane() {
  const { t } = useTranslation("email");
  const selectedThreadId = useSelectedThreadId();
  const selectedThread = useThreadStore((s) =>
    selectedThreadId ? (s.threadMap.get(selectedThreadId) ?? null) : null,
  );

  if (!selectedThread) {
    return (
      <div className="flex-1 flex flex-col bg-bg-primary/50 glass-panel">
        <EmptyState
          illustration={ReadingPaneIllustration}
          title={t("readingPaneTitle")}
          subtitle={t("readingPaneSubtitle")}
        />
      </div>
    );
  }

  return (
    <div className="flex-1 bg-bg-primary/50 overflow-hidden glass-panel">
      <ThreadView thread={selectedThread} />
    </div>
  );
}
