import { FolderSearch } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import { LABEL_MAP } from "@/hooks/useEmailListData";
import { useLabelStore } from "@/stores/labelStore";
import { useUILayoutStore } from "@/stores/uiLayoutStore";
import { CategoryTabs } from "../email/CategoryTabs";
import { SearchBar } from "../search/SearchBar";

export function EmailListHeader({
  activeLabel,
  activeCategory,
  setActiveCategory,
  inboxViewMode,
  isSmartFolder,
  activeSmartFolder,
  filteredThreadsCount,
  categoryUnreadCounts,
}: {
  activeLabel: string;
  activeCategory: string;
  setActiveCategory: (cat: string) => void;
  inboxViewMode: string;
  isSmartFolder: boolean;
  activeSmartFolder: { id: string; name: string; isDefault?: boolean } | null;
  filteredThreadsCount: number;
  categoryUnreadCounts: Map<string, number>;
}): React.ReactNode {
  const { t } = useTranslation("email");
  const readFilter = useUILayoutStore((s) => s.readFilter);
  const setReadFilter = useUILayoutStore((s) => s.setReadFilter);
  const userLabels = useLabelStore((s) => s.labels);

  return (
    <>
      {/* Search */}
      <div className="px-3 py-2 border-b border-border-secondary">
        <SearchBar />
      </div>

      {/* Header */}
      <div className="px-4 py-2 border-b border-border-primary flex items-center justify-between">
        <div>
          <h2 className="text-sm font-semibold text-text-primary capitalize flex items-center gap-1.5">
            {isSmartFolder && (
              <FolderSearch size={14} className="text-accent shrink-0" />
            )}
            {isSmartFolder
              ? ((activeSmartFolder?.isDefault
                  ? t(`sidebar:${activeSmartFolder.id}`, {
                      defaultValue: activeSmartFolder?.name,
                    })
                  : activeSmartFolder?.name) ?? "Smart Folder")
              : activeLabel === "inbox" &&
                  inboxViewMode === "split" &&
                  activeCategory !== "All"
                ? `Inbox — ${activeCategory}`
                : LABEL_MAP[activeLabel] !== undefined
                  ? activeLabel
                  : (userLabels.find((l) => l.id === activeLabel)?.name ??
                    activeLabel)}
          </h2>
          <span className="text-xs text-text-tertiary">
            {t("common:conversations", { count: filteredThreadsCount })}
          </span>
        </div>
        <select
          value={readFilter}
          // biome-ignore lint/nursery/useExplicitType: inline callback
          onChange={(e) =>
            setReadFilter(e.target.value as "all" | "read" | "unread")
          }
          className="text-xs bg-bg-tertiary text-text-secondary px-2 py-1 rounded border border-border-primary"
        >
          <option value="all">{t("allFilter")}</option>
          <option value="unread">{t("unreadFilter")}</option>
          <option value="read">{t("readFilter")}</option>
        </select>
      </div>

      {/* Category tabs (inbox + split mode only) */}
      {activeLabel === "inbox" && inboxViewMode === "split" && (
        <CategoryTabs
          activeCategory={activeCategory}
          onCategoryChange={setActiveCategory}
          unreadCounts={Object.fromEntries(categoryUnreadCounts)}
        />
      )}
    </>
  );
}
