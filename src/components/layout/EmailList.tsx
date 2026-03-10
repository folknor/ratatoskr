import type React from "react";
import { useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useSelectedThreadId } from "@/hooks/useRouteNavigation";
import { useEmailListData, PAGE_SIZE } from "@/hooks/useEmailListData";
import { navigateToThread } from "@/router/navigate";
import { invoke } from "@tauri-apps/api/core";
import { getMessagesForThread } from "@/core/queries";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { useContextMenuStore } from "@/stores/contextMenuStore";
import type { Thread } from "@/stores/threadStore";
import { useUILayoutStore } from "@/stores/uiLayoutStore";
import { ThreadCard } from "../email/ThreadCard";
import { EmailListSkeleton } from "../ui/Skeleton";
import { BundleRow } from "./BundleRow";
import { EmailListHeader } from "./EmailListHeader";
import { EmptyStateForContext } from "./EmptyStateForContext";
import { MultiSelectBar } from "./MultiSelectBar";

export function EmailList({
  width,
  listRef,
}: {
  width?: number;
  listRef?: React.Ref<HTMLDivElement>;
}): React.ReactNode {
  const { t } = useTranslation("email");
  const selectedThreadId = useSelectedThreadId();
  const readingPanePosition = useUILayoutStore((s) => s.readingPanePosition);
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const openMenu = useContextMenuStore((s) => s.openMenu);
  const openComposer = useComposerStore((s) => s.openComposer);

  const data = useEmailListData();
  const {
    threads,
    filteredThreads,
    visibleThreads,
    isLoading,
    hasMore,
    loadingMore,
    categoryMap,
    categoryUnreadCounts,
    followUpThreadIds,
    bundleRules,
    bundleSummaries,
    expandedBundles,
    setExpandedBundles,
    activeLabel,
    activeCategory,
    setActiveCategory,
    inboxViewMode,
    isSmartFolder,
    activeSmartFolder,
    readFilter,
    searchQuery,
    scrollContainerRef,
  } = data;

  const handleThreadContextMenu = useCallback(
    (e: React.MouseEvent, threadId: string) => {
      e.preventDefault();
      openMenu("thread", { x: e.clientX, y: e.clientY }, { threadId });
    },
    [openMenu],
  );

  const handleDraftClick = useCallback(
    async (thread: Thread) => {
      if (!activeAccountId) return;
      try {
        const messages = await getMessagesForThread(activeAccountId, thread.id);
        // Get the last message (the draft)
        const draftMsg = messages[messages.length - 1];
        if (!draftMsg) return;

        // Look up the Gmail draft ID so auto-save can update the existing draft
        let draftId: string | null = null;
        try {
          const drafts = await invoke<
            { id: string; message: { id: string; threadId: string } }[]
          >("gmail_list_drafts", { accountId: activeAccountId });
          const match = drafts.find((d) => d.message?.id === draftMsg.id);
          if (match) draftId = match.id;
        } catch {
          // If we can't get draft ID, composer will create a new draft on save
        }

        const to = draftMsg.to_addresses
          ? draftMsg.to_addresses
              .split(",")
              .map((a) => a.trim())
              .filter(Boolean)
          : [];
        const cc = draftMsg.cc_addresses
          ? draftMsg.cc_addresses
              .split(",")
              .map((a) => a.trim())
              .filter(Boolean)
          : [];
        const bcc = draftMsg.bcc_addresses
          ? draftMsg.bcc_addresses
              .split(",")
              .map((a) => a.trim())
              .filter(Boolean)
          : [];

        openComposer({
          mode: "new",
          to,
          cc,
          bcc,
          subject: draftMsg.subject ?? "",
          bodyHtml: draftMsg.body_html ?? draftMsg.body_text ?? "",
          threadId: thread.id,
          draftId,
        });
      } catch (err) {
        console.error("Failed to open draft:", err);
      }
    },
    [activeAccountId, openComposer],
  );

  const handleThreadClick = useCallback(
    (thread: Thread) => {
      if (activeLabel === "drafts") {
        void handleDraftClick(thread);
      } else {
        navigateToThread(thread.id);
      }
    },
    [activeLabel, handleDraftClick],
  );

  // Auto-scroll selected thread into view (triggered by keyboard navigation)
  useEffect(() => {
    if (!(selectedThreadId && scrollContainerRef.current)) return;
    const el = scrollContainerRef.current.querySelector(
      `[data-thread-id="${CSS.escape(selectedThreadId)}"]`,
    );
    if (el) {
      el.scrollIntoView({ block: "nearest" });
    }
  }, [selectedThreadId, scrollContainerRef]);

  return (
    <div
      ref={listRef}
      className={`flex flex-col bg-bg-secondary/50 glass-panel ${
        readingPanePosition === "right"
          ? "min-w-[240px] shrink-0"
          : readingPanePosition === "bottom"
            ? "w-full border-b border-border-primary h-[40%] min-h-[200px]"
            : "w-full flex-1"
      }`}
      style={readingPanePosition === "right" && width ? { width } : undefined}
    >
      <EmailListHeader
        activeLabel={activeLabel}
        activeCategory={activeCategory}
        setActiveCategory={setActiveCategory}
        inboxViewMode={inboxViewMode}
        isSmartFolder={isSmartFolder}
        activeSmartFolder={activeSmartFolder}
        filteredThreadsCount={filteredThreads.length}
        categoryUnreadCounts={categoryUnreadCounts}
      />

      <MultiSelectBar
        activeAccountId={activeAccountId}
        activeLabel={activeLabel}
        filteredThreadsCount={filteredThreads.length}
      />

      {/* Thread list */}
      <div ref={scrollContainerRef} className="flex-1 overflow-y-auto">
        {isLoading && threads.length === 0 ? (
          <EmailListSkeleton />
        ) : filteredThreads.length === 0 && bundleRules.length === 0 ? (
          <EmptyStateForContext
            searchQuery={searchQuery}
            activeAccountId={activeAccountId}
            activeLabel={activeLabel}
            readFilter={readFilter}
            activeCategory={activeCategory}
          />
        ) : (
          <>
            {/* Bundle rows for "All" inbox view */}
            {activeLabel === "inbox" &&
              activeCategory === "All" &&
              bundleRules.map((rule) => {
                const summary = bundleSummaries.get(rule.category);
                if (!summary || summary.count === 0) return null;
                const isExpanded = expandedBundles.has(rule.category);
                const bundledThreads = isExpanded
                  ? filteredThreads.filter(
                      (th) => categoryMap.get(th.id) === rule.category,
                    )
                  : [];
                return (
                  <BundleRow
                    key={`bundle-${rule.category}`}
                    rule={rule}
                    summary={summary}
                    isExpanded={isExpanded}
                    onToggle={(): void => {
                      setExpandedBundles((prev) => {
                        const next = new Set(prev);
                        if (next.has(rule.category)) next.delete(rule.category);
                        else next.add(rule.category);
                        return next;
                      });
                    }}
                    bundledThreads={bundledThreads}
                    selectedThreadId={selectedThreadId}
                    onThreadClick={handleThreadClick}
                    onContextMenu={handleThreadContextMenu}
                    followUpThreadIds={followUpThreadIds}
                  />
                );
              })}
            {visibleThreads.map((thread, idx) => {
              const prevThread = idx > 0 ? filteredThreads[idx - 1] : undefined;
              const showDivider =
                Boolean(prevThread?.isPinned) && !thread.isPinned;
              return (
                <div
                  key={thread.id}
                  data-thread-id={thread.id}
                  className={idx < 15 ? "stagger-in" : undefined}
                  style={
                    idx < 15 ? { animationDelay: `${idx * 30}ms` } : undefined
                  }
                >
                  {showDivider === true && (
                    <div className="px-4 py-1.5 text-xs font-medium text-text-tertiary uppercase tracking-wider bg-bg-tertiary/50 border-b border-border-secondary">
                      {t("otherEmails")}
                    </div>
                  )}
                  <ThreadCard
                    thread={thread}
                    isSelected={thread.id === selectedThreadId}
                    onClick={handleThreadClick}
                    onContextMenu={handleThreadContextMenu}
                    category={categoryMap.get(thread.id)}
                    showCategoryBadge={
                      activeLabel === "inbox" && activeCategory === "All"
                    }
                    hasFollowUp={followUpThreadIds.has(thread.id)}
                  />
                </div>
              );
            })}
            {loadingMore === true && (
              <div className="px-4 py-3 text-center text-xs text-text-tertiary">
                {t("common:loadingMore")}
              </div>
            )}
            {!hasMore && threads.length > PAGE_SIZE && (
              <div className="px-4 py-3 text-center text-xs text-text-tertiary">
                {t("allLoaded")}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
