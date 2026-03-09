import { Archive, Ban, Trash2, X } from "lucide-react";
import type React from "react";
import { useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { CSSTransition } from "react-transition-group";
import {
  deleteThread as deleteThreadFromDb,
  getGmailClient,
} from "@/core/mutations";
import { useThreadStore } from "@/stores/threadStore";

export function MultiSelectBar({
  activeAccountId,
  activeLabel,
  filteredThreadsCount,
}: {
  activeAccountId: string | null;
  activeLabel: string;
  filteredThreadsCount: number;
}): React.ReactNode {
  const { t } = useTranslation("email");
  const selectedThreadIds = useThreadStore((s) => s.selectedThreadIds);
  const removeThreads = useThreadStore((s) => s.removeThreads);
  const clearMultiSelect = useThreadStore((s) => s.clearMultiSelect);
  const selectAll = useThreadStore((s) => s.selectAll);
  const multiSelectCount = selectedThreadIds.size;
  const multiSelectBarRef = useRef<HTMLDivElement>(null);

  const handleBulkDelete = useCallback(async (): Promise<void> => {
    if (!activeAccountId || multiSelectCount === 0) return;
    const isTrashView = activeLabel === "trash";
    const ids = [...selectedThreadIds];
    removeThreads(ids);
    try {
      const client = await getGmailClient(activeAccountId);
      await Promise.all(
        ids.map(async (id) => {
          if (isTrashView) {
            await client.deleteThread(id);
            await deleteThreadFromDb(activeAccountId, id);
          } else {
            await client.modifyThread(id, ["TRASH"], ["INBOX"]);
          }
        }),
      );
    } catch (err) {
      console.error("Bulk delete failed:", err);
    }
  }, [
    activeAccountId,
    activeLabel,
    multiSelectCount,
    selectedThreadIds,
    removeThreads,
  ]);

  const handleBulkArchive = useCallback(async (): Promise<void> => {
    if (!activeAccountId || multiSelectCount === 0) return;
    const ids = [...selectedThreadIds];
    removeThreads(ids);
    try {
      const client = await getGmailClient(activeAccountId);
      await Promise.all(
        ids.map((id) => client.modifyThread(id, undefined, ["INBOX"])),
      );
    } catch (err) {
      console.error("Bulk archive failed:", err);
    }
  }, [activeAccountId, multiSelectCount, selectedThreadIds, removeThreads]);

  const handleBulkSpam = useCallback(async (): Promise<void> => {
    if (!activeAccountId || multiSelectCount === 0) return;
    const ids = [...selectedThreadIds];
    const isSpamView = activeLabel === "spam";
    removeThreads(ids);
    try {
      const client = await getGmailClient(activeAccountId);
      await Promise.all(
        ids.map((id) =>
          isSpamView
            ? client.modifyThread(id, ["INBOX"], ["SPAM"])
            : client.modifyThread(id, ["SPAM"], ["INBOX"]),
        ),
      );
    } catch (err) {
      console.error("Bulk spam failed:", err);
    }
  }, [
    activeAccountId,
    activeLabel,
    multiSelectCount,
    selectedThreadIds,
    removeThreads,
  ]);

  return (
    <CSSTransition
      nodeRef={multiSelectBarRef}
      in={multiSelectCount > 0}
      timeout={150}
      classNames="slide-down"
      unmountOnExit
    >
      <div
        ref={multiSelectBarRef}
        className="px-3 py-2 border-b border-border-primary bg-accent/5 flex items-center justify-between"
      >
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-text-primary">
            {multiSelectCount} {t("common:selected")}
          </span>
          {multiSelectCount < filteredThreadsCount && (
            <button
              type="button"
              onClick={selectAll}
              className="text-xs text-accent hover:text-accent-hover transition-colors"
            >
              {t("common:selectAll")}
            </button>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={handleBulkArchive}
            title={t("archiveSelected")}
            className="p-1.5 text-text-secondary hover:text-text-primary hover:bg-bg-hover rounded transition-colors"
          >
            <Archive size={14} />
          </button>
          <button
            type="button"
            onClick={handleBulkDelete}
            title={t("deleteSelected")}
            className="p-1.5 text-text-secondary hover:text-error hover:bg-bg-hover rounded transition-colors"
          >
            <Trash2 size={14} />
          </button>
          <button
            type="button"
            onClick={handleBulkSpam}
            title={activeLabel === "spam" ? t("notSpam") : t("reportSpam")}
            className="p-1.5 text-text-secondary hover:text-text-primary hover:bg-bg-hover rounded transition-colors"
          >
            <Ban size={14} />
          </button>
          <button
            type="button"
            onClick={clearMultiSelect}
            title={t("common:clearSelection")}
            className="p-1.5 text-text-secondary hover:text-text-primary hover:bg-bg-hover rounded transition-colors"
          >
            <X size={14} />
          </button>
        </div>
      </div>
    </CSSTransition>
  );
}
