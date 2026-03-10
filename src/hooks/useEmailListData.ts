import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useActiveCategory, useActiveLabel } from "@/hooks/useRouteNavigation";
import { navigateToLabel } from "@/router/navigate";
import {
  type DbBundleRule,
  getBundleRules,
  getBundleSummaries,
  getActiveFollowUpThreadIds,
  getCategoriesForThreads,
  getCategoryUnreadCounts,
  getHeldThreadIds,
  getSmartFolderSearchQuery,
  getThreadLabelIds,
  getThreadsForAccount,
  getThreadsForCategory,
  mapSmartFolderRows,
  querySmartFolderThreads,
  type SmartFolderRow,
} from "@/core/queries";
import { useAccountStore } from "@/stores/accountStore";
import { useSmartFolderStore } from "@/stores/smartFolderStore";
import { type Thread, useThreadStore } from "@/stores/threadStore";
import { useUILayoutStore } from "@/stores/uiLayoutStore";
import { useUIPreferencesStore } from "@/stores/uiPreferencesStore";

const PAGE_SIZE = 50;

// Map sidebar labels to Gmail label IDs
const LABEL_MAP: Record<string, string> = {
  inbox: "INBOX",
  starred: "STARRED",
  sent: "SENT",
  drafts: "DRAFT",
  trash: "TRASH",
  spam: "SPAM",
  snoozed: "SNOOZED",
  all: "", // no filter
};

export { LABEL_MAP, PAGE_SIZE };

export interface EmailListData {
  // Core data
  threads: Thread[];
  filteredThreads: Thread[];
  visibleThreads: Thread[];
  isLoading: boolean;
  hasMore: boolean;
  loadingMore: boolean;

  // Metadata
  categoryMap: Map<string, string>;
  categoryUnreadCounts: Map<string, number>;
  followUpThreadIds: Set<string>;
  bundleRules: DbBundleRule[];
  bundleSummaries: Map<
    string,
    {
      count: number;
      latestSubject: string | null;
      latestSender: string | null;
    }
  >;
  heldThreadIds: Set<string>;
  expandedBundles: Set<string>;
  setExpandedBundles: React.Dispatch<React.SetStateAction<Set<string>>>;

  // Navigation / context
  activeAccountId: string | null;
  activeLabel: string;
  activeCategory: string;
  setActiveCategory: (cat: string) => void;
  inboxViewMode: string;
  isSmartFolder: boolean;
  activeSmartFolder:
    | ReturnType<typeof useSmartFolderStore.getState>["folders"][number]
    | null;
  readFilter: string;
  searchQuery: string | null;

  // Scroll
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
}

export function useEmailListData(): EmailListData {
  const threads = useThreadStore((s) => s.threads);
  const isLoading = useThreadStore((s) => s.isLoading);
  const setThreads = useThreadStore((s) => s.setThreads);
  const setLoading = useThreadStore((s) => s.setLoading);
  const searchThreadIds = useThreadStore((s) => s.searchThreadIds);
  const searchQuery = useThreadStore((s) => s.searchQuery);
  const clearSearch = useThreadStore((s) => s.clearSearch);

  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const activeLabel = useActiveLabel();
  const readFilter = useUILayoutStore((s) => s.readFilter);
  const smartFolders = useSmartFolderStore((s) => s.folders);
  const inboxViewMode = useUIPreferencesStore((s) => s.inboxViewMode);
  const routerCategory = useActiveCategory();

  // Smart folder detection
  const isSmartFolder = activeLabel.startsWith("smart-folder:");
  const smartFolderId = isSmartFolder
    ? activeLabel.replace("smart-folder:", "")
    : null;
  const activeSmartFolder = smartFolderId
    ? (smartFolders.find((f) => f.id === smartFolderId) ?? null)
    : null;

  // In split mode, use the router's category; in unified mode, always use "All"
  const activeCategory = inboxViewMode === "split" ? routerCategory : "All";
  const setActiveCategory =
    inboxViewMode === "split"
      ? (cat: string): void => navigateToLabel("inbox", { category: cat })
      : (): void => {};

  const [hasMore, setHasMore] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const [categoryMap, setCategoryMap] = useState<Map<string, string>>(
    () => new Map(),
  );
  const [categoryUnreadCounts, setCategoryUnreadCounts] = useState<
    Map<string, number>
  >(() => new Map());
  const [followUpThreadIds, setFollowUpThreadIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [bundleRules, setBundleRules] = useState<DbBundleRule[]>([]);
  const [heldThreadIds, setHeldThreadIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [expandedBundles, setExpandedBundles] = useState<Set<string>>(
    () => new Set(),
  );
  const [bundleSummaries, setBundleSummaries] = useState<
    Map<
      string,
      {
        count: number;
        latestSubject: string | null;
        latestSender: string | null;
      }
    >
  >(() => new Map());

  // Filtered threads (search + read filter)
  const filteredThreads = useMemo(() => {
    let filtered = threads;
    if (searchThreadIds !== null) {
      filtered = filtered.filter((th) => searchThreadIds.has(th.id));
    }
    if (readFilter === "unread") filtered = filtered.filter((th) => !th.isRead);
    else if (readFilter === "read")
      filtered = filtered.filter((th) => th.isRead);
    return filtered;
  }, [threads, readFilter, searchThreadIds]);

  // Pre-compute bundled category Set for O(1) lookups in filter
  const bundledCategorySet = useMemo(
    () => new Set(bundleRules.map((r) => r.category)),
    [bundleRules],
  );

  // Memoize visible threads (excludes bundled/held threads in "All" inbox view)
  const visibleThreads = useMemo(() => {
    if (activeLabel !== "inbox" || activeCategory !== "All")
      return filteredThreads;
    return filteredThreads.filter((th) => {
      const cat = categoryMap.get(th.id);
      if (cat && bundledCategorySet.has(cat)) return false;
      if (heldThreadIds.has(th.id)) return false;
      return true;
    });
  }, [
    filteredThreads,
    activeLabel,
    activeCategory,
    categoryMap,
    bundledCategorySet,
    heldThreadIds,
  ]);

  const mapDbThreads = useCallback(
    async (
      dbThreads: Awaited<ReturnType<typeof getThreadsForAccount>>,
    ): Promise<Thread[]> =>
      Promise.all(
        dbThreads.map(async (row) => {
          const labelIds = await getThreadLabelIds(row.account_id, row.id);
          return {
            id: row.id,
            accountId: row.account_id,
            subject: row.subject,
            snippet: row.snippet,
            lastMessageAt: row.last_message_at ?? 0,
            messageCount: row.message_count,
            isRead: Boolean(row.is_read),
            isStarred: Boolean(row.is_starred),
            isPinned: Boolean(row.is_pinned),
            isMuted: Boolean(row.is_muted),
            hasAttachments: Boolean(row.has_attachments),
            labelIds,
            fromName: row.from_name,
            fromAddress: row.from_address,
          };
        }),
      ),
    [activeAccountId],
  );

  const loadThreads = useCallback(async () => {
    if (!activeAccountId) {
      setThreads([]);
      return;
    }

    clearSearch();
    setLoading(true);
    setHasMore(true);
    try {
      // Smart folder query path
      if (isSmartFolder && activeSmartFolder) {
        const { sql, params } = getSmartFolderSearchQuery(
          activeSmartFolder.query,
          activeAccountId,
          PAGE_SIZE,
        );
        const rows = await querySmartFolderThreads<SmartFolderRow>(sql, params);
        const mapped = await mapSmartFolderRows(rows);
        setThreads(mapped);
        setHasMore(false); // Smart folders load all at once
      } else {
        let dbThreads: Awaited<ReturnType<typeof getThreadsForAccount>>;
        // Server-side category filtering for inbox
        if (activeLabel === "inbox" && activeCategory !== "All") {
          dbThreads = await getThreadsForCategory(
            activeAccountId,
            activeCategory,
            PAGE_SIZE,
            0,
          );
        } else {
          const gmailLabelId = LABEL_MAP[activeLabel] ?? activeLabel;
          dbThreads = await getThreadsForAccount(
            activeAccountId,
            gmailLabelId || undefined,
            PAGE_SIZE,
            0,
          );
        }

        const mapped = await mapDbThreads(dbThreads);
        setThreads(mapped);
        setHasMore(dbThreads.length === PAGE_SIZE);
      }
    } catch (err) {
      console.error("Failed to load threads:", err);
    } finally {
      setLoading(false);
    }
  }, [
    activeAccountId,
    activeLabel,
    activeCategory,
    isSmartFolder,
    activeSmartFolder,
    setThreads,
    setLoading,
    mapDbThreads,
    clearSearch,
  ]);

  const loadMore = useCallback(async () => {
    if (!activeAccountId || loadingMore || !hasMore) return;

    setLoadingMore(true);
    try {
      const offset = threads.length;
      let dbThreads: Awaited<ReturnType<typeof getThreadsForAccount>>;
      if (activeLabel === "inbox" && activeCategory !== "All") {
        dbThreads = await getThreadsForCategory(
          activeAccountId,
          activeCategory,
          PAGE_SIZE,
          offset,
        );
      } else {
        const gmailLabelId = LABEL_MAP[activeLabel] ?? activeLabel;
        dbThreads = await getThreadsForAccount(
          activeAccountId,
          gmailLabelId || undefined,
          PAGE_SIZE,
          offset,
        );
      }

      const mapped = await mapDbThreads(dbThreads);
      if (mapped.length > 0) {
        setThreads([...threads, ...mapped]);
      }
      setHasMore(dbThreads.length === PAGE_SIZE);
    } catch (err) {
      console.error("Failed to load more threads:", err);
    } finally {
      setLoadingMore(false);
    }
  }, [
    activeAccountId,
    activeLabel,
    activeCategory,
    threads,
    loadingMore,
    hasMore,
    setThreads,
    mapDbThreads,
  ]);

  // Load threads on mount and when dependencies change
  useEffect(() => {
    void loadThreads();
  }, [loadThreads]);

  // Stable thread ID key — only changes when the actual set of thread IDs changes
  const threadIdKey = useMemo(
    () => threads.map((th) => th.id).join(","),
    [threads],
  );

  // Load all thread metadata in one coordinated effect
  useEffect(() => {
    let cancelled = false;

    if (!activeAccountId) {
      setCategoryMap(new Map());
      setCategoryUnreadCounts(new Map());
      setFollowUpThreadIds(new Set());
      setBundleRules([]);
      setHeldThreadIds(new Set());
      setBundleSummaries(new Map());
      return;
    }

    const threadIds = threadIdKey ? threadIdKey.split(",") : [];
    const isInbox = activeLabel === "inbox";
    const isAllCategory = activeCategory === "All";

    const loadMetadata = async (): Promise<void> => {
      try {
        const promises: Promise<void>[] = [];

        // Categories (only for inbox "All" tab with threads)
        if (isInbox && isAllCategory && threadIds.length > 0) {
          promises.push(
            getCategoriesForThreads(activeAccountId, threadIds).then(
              (result) => {
                if (!cancelled) setCategoryMap(result);
              },
            ),
          );
        } else {
          setCategoryMap(new Map());
        }

        // Unread counts (only for inbox)
        if (isInbox) {
          promises.push(
            getCategoryUnreadCounts(activeAccountId).then((result) => {
              if (!cancelled) setCategoryUnreadCounts(result);
            }),
          );
        } else {
          setCategoryUnreadCounts(new Map());
        }

        // Follow-up indicators
        if (threadIds.length > 0) {
          promises.push(
            getActiveFollowUpThreadIds(activeAccountId, threadIds)
              .then((result) => {
                if (!cancelled) setFollowUpThreadIds(result);
              })
              .catch(() => {
                if (!cancelled) setFollowUpThreadIds(new Set());
              }),
          );
        } else {
          setFollowUpThreadIds(new Set());
        }

        // Bundle rules + held threads (only for inbox)
        if (isInbox) {
          promises.push(
            getBundleRules(activeAccountId)
              .then(async (rules) => {
                if (cancelled) return;
                const bundled = rules.filter((r) => r.is_bundled);
                setBundleRules(bundled);
                if (bundled.length > 0) {
                  const summaries = await getBundleSummaries(
                    activeAccountId,
                    bundled.map((r) => r.category),
                  ).catch(() => new Map());
                  if (!cancelled) setBundleSummaries(summaries);
                } else {
                  if (!cancelled) setBundleSummaries(new Map());
                }
              })
              .catch(() => {
                if (!cancelled) setBundleRules([]);
              }),
          );
          promises.push(
            getHeldThreadIds(activeAccountId)
              .then((result) => {
                if (!cancelled) setHeldThreadIds(result);
              })
              .catch(() => {
                if (!cancelled) setHeldThreadIds(new Set());
              }),
          );
        } else {
          setBundleRules([]);
          setHeldThreadIds(new Set());
          setBundleSummaries(new Map());
        }

        await Promise.all(promises);
      } catch (err) {
        console.error("Failed to load thread metadata:", err);
      }
    };

    void loadMetadata();
    return (): void => {
      cancelled = true;
    };
  }, [threadIdKey, activeLabel, activeCategory, activeAccountId]);

  // Listen for sync completion to reload
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const handler = (): void => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void loadThreads(), 500);
    };
    window.addEventListener("ratatoskr-sync-done", handler);
    return (): void => {
      window.removeEventListener("ratatoskr-sync-done", handler);
      if (timer) clearTimeout(timer);
    };
  }, [loadThreads]);

  // Infinite scroll: load more when near bottom
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    const handleScroll = (): void => {
      const { scrollTop, scrollHeight, clientHeight } = container;
      if (scrollHeight - scrollTop - clientHeight < 200) {
        void loadMore();
      }
    };

    container.addEventListener("scroll", handleScroll, { passive: true });
    return (): void => container.removeEventListener("scroll", handleScroll);
  }, [loadMore]);

  return {
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
    heldThreadIds,
    expandedBundles,
    setExpandedBundles,
    activeAccountId,
    activeLabel,
    activeCategory,
    setActiveCategory,
    inboxViewMode,
    isSmartFolder,
    activeSmartFolder,
    readFilter,
    searchQuery,
    scrollContainerRef,
  };
}
