import { Outlet } from "@tanstack/react-router";
import { invoke } from "@tauri-apps/api/core";
import type React from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AddAccount } from "./components/accounts/AddAccount";
import { Composer } from "./components/composer/Composer";
import { UndoSendToast } from "./components/composer/UndoSendToast";
import { DndProvider } from "./components/dnd/DndProvider";
import { MoveToFolderDialog } from "./components/email/MoveToFolderDialog";
import { Sidebar } from "./components/layout/Sidebar";
import { TitleBar } from "./components/layout/TitleBar";
import { AskInbox } from "./components/search/AskInbox";
import { CommandPalette } from "./components/search/CommandPalette";
import { ShortcutsHelp } from "./components/search/ShortcutsHelp";
import { ContextMenuPortal } from "./components/ui/ContextMenuPortal";
import { ErrorBoundary } from "./components/ui/ErrorBoundary";
import { OfflineBanner } from "./components/ui/OfflineBanner";
import { UpdateToast } from "./components/ui/UpdateToast";
import type { ColorThemeId } from "./constants/themes";
import { COLOR_THEMES, getThemeById } from "./constants/themes";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { router } from "./router";
import { getSelectedThreadId } from "./router/navigate";
import {
  startPreCacheManager,
  stopPreCacheManager,
} from "./services/attachments/preCacheManager";
import { updateBadgeCount } from "./services/badgeManager";
import {
  startBundleChecker,
  stopBundleChecker,
} from "./services/bundles/bundleManager";
import { getAllAccounts } from "./services/db/accounts";
import { runMigrations } from "./services/db/migrations";
import { getSetting } from "./services/db/settings";
import { getIncompleteTaskCount } from "./services/db/tasks";
import { initDeepLinkHandler } from "./services/deepLinkHandler";
import {
  startFollowUpChecker,
  stopFollowUpChecker,
} from "./services/followup/followupManager";
import {
  initGlobalShortcut,
  unregisterComposeShortcut,
} from "./services/globalShortcut";
import { fetchSendAsAliases } from "./services/gmail/sendAs";
import {
  onSyncStatus,
  startBackgroundSync,
  stopBackgroundSync,
  syncAccount,
  triggerSync,
} from "./services/gmail/syncManager";
import { initializeClients } from "./services/gmail/tokenManager";
import { initNotifications } from "./services/notifications/notificationManager";
import {
  startQueueProcessor,
  stopQueueProcessor,
  triggerQueueFlush,
} from "./services/queue/queueProcessor";
import {
  startScheduledSendChecker,
  stopScheduledSendChecker,
} from "./services/snooze/scheduledSendManager";
import {
  startSnoozeChecker,
  stopSnoozeChecker,
} from "./services/snooze/snoozeManager";
import {
  startUpdateChecker,
  stopUpdateChecker,
} from "./services/updateManager";
import { useAccountStore } from "./stores/accountStore";
import { useShortcutStore } from "./stores/shortcutStore";
import { useSyncStateStore } from "./stores/syncStateStore";
import { useTaskStore } from "./stores/taskStore";
import { useUILayoutStore } from "./stores/uiLayoutStore";
import { useUIPreferencesStore } from "./stores/uiPreferencesStore";
import { formatSyncError } from "./utils/networkErrors";

/**
 * Sync bridge: subscribes to router state changes and writes the selected
 * thread ID to the threadStore so that range-select and other multi-select
 * logic can use it as an anchor.
 */
function useRouterSyncBridge(): void {
  useEffect(
    () =>
      router.subscribe("onResolved", () => {
        const threadId = getSelectedThreadId();
        if (useThreadStore.getState().selectedThreadId !== threadId) {
          useThreadStore.getState().selectThread(threadId);
        }
      }),
    [],
  );
}

import { useThreadStore } from "./stores/threadStore";

export default function App(): React.ReactNode {
  const { t } = useTranslation();
  const theme = useUIPreferencesStore((s) => s.theme);
  const fontScale = useUIPreferencesStore((s) => s.fontScale);
  const colorTheme = useUIPreferencesStore((s) => s.colorTheme);
  const reduceMotion = useUIPreferencesStore((s) => s.reduceMotion);
  const showSyncStatusBar = useUIPreferencesStore((s) => s.showSyncStatusBar);
  const sidebarCollapsed = useUILayoutStore((s) => s.sidebarCollapsed);
  const [showAddAccount, setShowAddAccount] = useState(false);
  const [initialized, setInitialized] = useState(false);
  const [syncStatus, setSyncStatus] = useState<string | null>(null);
  const [showCommandPalette, setShowCommandPalette] = useState(false);
  const [showShortcutsHelp, setShowShortcutsHelp] = useState(false);
  const [showAskInbox, setShowAskInbox] = useState(false);
  const [moveToFolderState, setMoveToFolderState] = useState<{
    open: boolean;
    threadIds: string[];
  }>({ open: false, threadIds: [] });
  const deepLinkCleanupRef = useRef<(() => void) | undefined>(undefined);

  // Sync bridge: router state → Zustand stores (temporary)
  useRouterSyncBridge();

  // Register global keyboard shortcuts
  useKeyboardShortcuts();

  // Network status detection
  useEffect(() => {
    const { setOnline } = useSyncStateStore.getState();
    setOnline(navigator.onLine);

    const handleOnline = (): void => {
      setOnline(true);
      void triggerQueueFlush();
      const accounts = useAccountStore.getState().accounts;
      const activeIds = accounts.filter((a) => a.isActive).map((a) => a.id);
      if (activeIds.length > 0) void triggerSync(activeIds);
    };
    const handleOffline = (): void => setOnline(false);

    window.addEventListener("online", handleOnline);
    window.addEventListener("offline", handleOffline);
    return (): void => {
      window.removeEventListener("online", handleOnline);
      window.removeEventListener("offline", handleOffline);
    };
  }, []);

  // Suppress default browser context menu globally (Tauri app should feel native)
  // Elements with data-native-context-menu opt out so the browser menu is available
  useEffect(() => {
    const handler = (e: MouseEvent): void => {
      if ((e.target as HTMLElement).closest?.("[data-native-context-menu]"))
        return;
      e.preventDefault();
    };
    document.addEventListener("contextmenu", handler);
    return (): void => document.removeEventListener("contextmenu", handler);
  }, []);

  // Listen for command palette / shortcuts help toggle events
  useEffect(() => {
    const togglePalette = (): void => setShowCommandPalette((p) => !p);
    const toggleHelp = (): void => setShowShortcutsHelp((p) => !p);
    const toggleAskInbox = (): void => setShowAskInbox((p) => !p);
    const handleMoveToFolder = (e: Event): void => {
      const detail = (e as CustomEvent<{ threadIds: string[] }>).detail;
      setMoveToFolderState({ open: true, threadIds: detail.threadIds });
    };
    window.addEventListener("ratatoskr-toggle-command-palette", togglePalette);
    window.addEventListener("ratatoskr-toggle-shortcuts-help", toggleHelp);
    window.addEventListener("ratatoskr-toggle-ask-inbox", toggleAskInbox);
    window.addEventListener("ratatoskr-move-to-folder", handleMoveToFolder);
    return (): void => {
      window.removeEventListener(
        "ratatoskr-toggle-command-palette",
        togglePalette,
      );
      window.removeEventListener("ratatoskr-toggle-shortcuts-help", toggleHelp);
      window.removeEventListener("ratatoskr-toggle-ask-inbox", toggleAskInbox);
      window.removeEventListener(
        "ratatoskr-move-to-folder",
        handleMoveToFolder,
      );
    };
  }, []);

  // Listen for tray "Check for Mail" button
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(({ listen }) => {
      void listen("tray-check-mail", () => {
        const accounts = useAccountStore.getState().accounts;
        const activeIds = accounts.filter((a) => a.isActive).map((a) => a.id);
        if (activeIds.length > 0) {
          void triggerSync(activeIds);
        }
      }).then((fn) => {
        unlisten = fn;
      });
    });
    return (): void => {
      unlisten?.();
    };
  }, []);

  // Initialize database, load accounts, start sync
  useEffect(() => {
    async function init(): Promise<void> {
      try {
        await runMigrations();

        // Migrate existing bodies from metadata DB to compressed body store (Phase 2).
        // This is idempotent — once all bodies are migrated, it's a no-op.
        import("@/core/rustDb").then(({ bodyStoreMigrate }) =>
          bodyStoreMigrate().catch((e: unknown) =>
            console.warn("Body store migration skipped:", e),
          ),
        );

        // Bootstrap tantivy search index on first run (Phase 3).
        // Uses a version key so the index is rebuilt when the schema changes.
        {
          const SEARCH_INDEX_VERSION = "1";
          const indexVersion = await getSetting("search_index_version");
          if (indexVersion !== SEARCH_INDEX_VERSION) {
            import("@/core/rustDb").then(async (rustDb) => {
              try {
                const count = await rustDb.rebuildSearchIndex();
                await rustDb.setSetting(
                  "search_index_version",
                  SEARCH_INDEX_VERSION,
                );
                console.log(`Search index built: ${String(count)} documents`);
              } catch (e) {
                console.warn("Search index rebuild failed:", e);
              }
            });
          }
        }

        // Load persisted language (must be after migrations, before UI renders)
        const { loadPersistedLanguage } = await import("./i18n");
        await loadPersistedLanguage();

        const layout = useUILayoutStore.getState();
        const prefs = useUIPreferencesStore.getState();

        // Restore persisted theme
        const savedTheme = await getSetting("theme");
        if (
          savedTheme === "light" ||
          savedTheme === "dark" ||
          savedTheme === "system"
        ) {
          prefs.setTheme(savedTheme);
        }

        // Restore persisted sidebar state
        const savedSidebar = await getSetting("sidebar_collapsed");
        if (savedSidebar === "true") {
          layout.setSidebarCollapsed(true);
        }

        // Restore contact sidebar visibility
        const savedContactSidebar = await getSetting("contact_sidebar_visible");
        if (savedContactSidebar === "false") {
          layout.setContactSidebarVisible(false);
        }

        // Restore reading pane position
        const savedPanePos = await getSetting("reading_pane_position");
        if (
          savedPanePos === "right" ||
          savedPanePos === "bottom" ||
          savedPanePos === "hidden"
        ) {
          layout.setReadingPanePosition(savedPanePos);
        }

        // Restore read filter
        const savedReadFilter = await getSetting("read_filter");
        if (
          savedReadFilter === "all" ||
          savedReadFilter === "read" ||
          savedReadFilter === "unread"
        ) {
          layout.setReadFilter(savedReadFilter);
        }

        // Restore email list width
        const savedListWidth = await getSetting("email_list_width");
        if (savedListWidth) {
          const w = parseInt(savedListWidth, 10);
          if (w >= 240 && w <= 800) layout.setEmailListWidth(w);
        }

        // Restore email density
        const savedDensity = await getSetting("email_density");
        if (
          savedDensity === "compact" ||
          savedDensity === "default" ||
          savedDensity === "spacious"
        ) {
          prefs.setEmailDensity(savedDensity);
        }

        // Restore default reply mode
        const savedReplyMode = await getSetting("default_reply_mode");
        if (savedReplyMode === "reply" || savedReplyMode === "replyAll") {
          prefs.setDefaultReplyMode(savedReplyMode);
        }

        // Restore mark-as-read behavior
        const savedMarkRead = await getSetting("mark_as_read_behavior");
        if (
          savedMarkRead === "instant" ||
          savedMarkRead === "2s" ||
          savedMarkRead === "manual"
        ) {
          prefs.setMarkAsReadBehavior(savedMarkRead);
        }

        // Restore send and archive
        const savedSendArchive = await getSetting("send_and_archive");
        if (savedSendArchive === "true") {
          prefs.setSendAndArchive(true);
        }

        // Restore font scale
        const savedFontScale = await getSetting("font_size");
        if (
          savedFontScale === "small" ||
          savedFontScale === "default" ||
          savedFontScale === "large" ||
          savedFontScale === "xlarge"
        ) {
          prefs.setFontScale(savedFontScale);
        }

        // Restore color theme
        const savedColorTheme = await getSetting("color_theme");
        if (
          savedColorTheme &&
          COLOR_THEMES.some((ct) => ct.id === savedColorTheme)
        ) {
          prefs.setColorTheme(savedColorTheme as ColorThemeId);
        }

        // Restore inbox view mode
        const savedViewMode = await getSetting("inbox_view_mode");
        if (savedViewMode === "unified" || savedViewMode === "split") {
          prefs.setInboxViewMode(savedViewMode);
        }

        // Restore reduce motion preference
        const savedReduceMotion = await getSetting("reduce_motion");
        if (savedReduceMotion === "true") {
          prefs.setReduceMotion(true);
        }

        // Restore show sync status bar preference
        const savedShowSyncStatus = await getSetting("show_sync_status");
        if (savedShowSyncStatus === "false") {
          prefs.setShowSyncStatusBar(false);
        }

        // Restore task sidebar visibility
        const savedTaskSidebar = await getSetting("task_sidebar_visible");
        if (savedTaskSidebar === "true") {
          layout.setTaskSidebarVisible(true);
        }

        // Restore sidebar nav config
        const savedNavConfig = await getSetting("sidebar_nav_config");
        if (savedNavConfig) {
          try {
            const parsed = JSON.parse(savedNavConfig);
            if (Array.isArray(parsed)) layout.restoreSidebarNavConfig(parsed);
          } catch {
            /* ignore malformed JSON */
          }
        }

        // Load custom keyboard shortcuts
        await useShortcutStore.getState().loadKeyMap();

        const dbAccounts = await getAllAccounts();
        const mapped = dbAccounts.map((a) => ({
          id: a.id,
          email: a.email,
          displayName: a.display_name,
          avatarUrl: a.avatar_url,
          isActive: a.is_active === 1,
          provider: a.provider,
        }));
        const savedAccountId = await getSetting("active_account_id");
        useAccountStore.getState().setAccounts(mapped, savedAccountId);

        // Initialize Gmail clients for existing accounts
        await initializeClients();

        // Initialize JMAP clients
        for (const account of dbAccounts.filter(
          (a) => a.is_active && a.provider === "jmap",
        )) {
          try {
            await invoke("jmap_init_client", { accountId: account.id });
          } catch (err) {
            console.warn(`Failed to init JMAP client for ${account.id}:`, err);
          }
        }

        // Initialize Graph clients
        for (const account of dbAccounts.filter(
          (a) => a.is_active && a.provider === "graph",
        )) {
          try {
            await invoke("graph_init_client", { accountId: account.id });
          } catch (err) {
            console.warn(`Failed to init Graph client for ${account.id}:`, err);
          }
        }

        // Fetch send-as aliases for each active email account (skip CalDAV-only)
        const activeIds = mapped.filter((a) => a.isActive).map((a) => a.id);
        const emailAccountIds = mapped
          .filter((a) => a.isActive && a.provider !== "caldav")
          .map((a) => a.id);
        for (const accountId of emailAccountIds) {
          try {
            await fetchSendAsAliases(accountId);
          } catch (err) {
            console.warn(
              `Failed to fetch send-as aliases for ${accountId}:`,
              err,
            );
          }
        }

        // Start background sync for active accounts
        if (activeIds.length > 0) {
          startBackgroundSync(activeIds);
        }

        // Start snooze, scheduled send, follow-up, bundle, and queue checkers
        startSnoozeChecker();
        startScheduledSendChecker();
        startFollowUpChecker();
        startBundleChecker();
        startQueueProcessor();
        startPreCacheManager();

        // Initialize notifications
        await initNotifications();

        // Initialize global compose shortcut
        await initGlobalShortcut();

        // Initialize deep link handler
        deepLinkCleanupRef.current = await initDeepLinkHandler();

        // Initial badge count
        await updateBadgeCount();

        // Load initial task count
        const activeAcct = useAccountStore.getState().activeAccountId;
        if (activeAcct) {
          const count = await getIncompleteTaskCount(activeAcct);
          useTaskStore.getState().setIncompleteCount(count);
        }

        // Start auto-update checker
        startUpdateChecker();
      } catch (err) {
        console.error("Failed to initialize:", err);
      }
      setInitialized(true);
      invoke("close_splashscreen").catch(() => {});
    }

    void init();

    return (): void => {
      stopBackgroundSync();
      stopSnoozeChecker();
      stopScheduledSendChecker();
      stopFollowUpChecker();
      stopBundleChecker();
      stopQueueProcessor();
      stopPreCacheManager();
      stopUpdateChecker();
      void unregisterComposeShortcut();
      deepLinkCleanupRef.current?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- store setters are stable references
  }, []);

  // Listen for sync status updates
  const backfillDoneRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    const unsub = onSyncStatus((accountId, status, progress, error) => {
      if (status === "syncing") {
        if (progress) {
          if (progress.phase === "messages") {
            setSyncStatus(
              `Syncing: ${progress.current}/${progress.total} messages`,
            );
          } else if (progress.phase === "labels") {
            setSyncStatus("Syncing labels...");
          } else if (progress.phase === "threads") {
            setSyncStatus(
              `Building threads... (${progress.current}/${progress.total})`,
            );
          }
        } else {
          setSyncStatus("Syncing...");
        }
      } else if (status === "done") {
        setSyncStatus(null);
        window.dispatchEvent(new Event("ratatoskr-sync-done"));
        void updateBadgeCount();

        // Backfill uncategorized threads after first successful sync per account
        if (!backfillDoneRef.current.has(accountId)) {
          backfillDoneRef.current.add(accountId);
          import("./services/categorization/backfillService")
            .then(({ backfillUncategorizedThreads }) =>
              backfillUncategorizedThreads(accountId),
            )
            .catch((err) => console.error("Backfill error:", err));
        }
      } else if (status === "error") {
        setSyncStatus(
          error ? `Sync failed: ${formatSyncError(error)}` : "Sync failed",
        );
        // Still dispatch sync-done so the UI refreshes with any partially stored data
        window.dispatchEvent(new Event("ratatoskr-sync-done"));
        // Auto-clear the error after 8 seconds
        setTimeout(() => setSyncStatus(null), 8_000);
      }
    });
    return unsub;
  }, []);

  // Sync theme class to <html> element
  useEffect((): (() => void) | undefined => {
    const root = document.documentElement;
    if (theme === "dark") {
      root.classList.add("dark");
      return;
    } else if (theme === "light") {
      root.classList.remove("dark");
      return;
    } else {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      const apply = (): void => {
        if (mq.matches) {
          root.classList.add("dark");
        } else {
          root.classList.remove("dark");
        }
      };
      apply();
      mq.addEventListener("change", apply);
      return (): void => mq.removeEventListener("change", apply);
    }
  }, [theme]);

  // Sync font-scale class to <html> element
  useEffect((): void => {
    const root = document.documentElement;
    root.classList.remove(
      "font-scale-small",
      "font-scale-default",
      "font-scale-large",
      "font-scale-xlarge",
    );
    root.classList.add(`font-scale-${fontScale}`);
  }, [fontScale]);

  // Sync reduce-motion class to <html> element
  useEffect((): void => {
    const root = document.documentElement;
    root.classList.toggle("reduce-motion", reduceMotion);
  }, [reduceMotion]);

  // Apply color theme CSS custom properties to <html>
  useEffect((): (() => void) | undefined => {
    const root = document.documentElement;
    const props = [
      "--color-accent",
      "--color-accent-hover",
      "--color-accent-light",
      "--color-bg-selected",
      "--color-sidebar-active",
    ];

    const apply = (): void => {
      if (colorTheme === "indigo") {
        // Default theme — remove inline overrides, let CSS handle it
        for (const p of props) root.style.removeProperty(p);
        return;
      }
      const themeData = getThemeById(colorTheme);
      const isDark =
        theme === "dark" ||
        (theme === "system" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches);
      const colors = isDark ? themeData.dark : themeData.light;
      root.style.setProperty("--color-accent", colors.accent);
      root.style.setProperty("--color-accent-hover", colors.accentHover);
      root.style.setProperty("--color-accent-light", colors.accentLight);
      root.style.setProperty("--color-bg-selected", colors.bgSelected);
      root.style.setProperty("--color-sidebar-active", colors.sidebarActive);
    };

    apply();

    if (theme === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      mq.addEventListener("change", apply);
      return (): void => mq.removeEventListener("change", apply);
    }
    return;
  }, [colorTheme, theme]);

  const handleAddAccountSuccess = useCallback(async () => {
    setShowAddAccount(false);
    const dbAccounts = await getAllAccounts();
    const mapped = dbAccounts.map((a) => ({
      id: a.id,
      email: a.email,
      displayName: a.display_name,
      avatarUrl: a.avatar_url,
      isActive: a.is_active === 1,
      provider: a.provider,
    }));
    useAccountStore.getState().setAccounts(mapped);

    // Re-initialize clients for the new account
    await initializeClients();

    const newest = mapped[mapped.length - 1];
    if (newest) {
      // Sync the new account immediately — before restarting the background
      // timer so it doesn't queue behind delta syncs for existing accounts.
      await syncAccount(newest.id);

      // Fetch send-as aliases in the background (non-blocking, skip CalDAV-only accounts)
      if (newest.provider !== "caldav") {
        fetchSendAsAliases(newest.id).catch((err) =>
          console.warn(`Failed to fetch send-as aliases for new account:`, err),
        );
      }
    }

    // Restart background sync for all accounts, but skip the immediate run
    // since we already triggered the new account's sync above.
    const activeIds = mapped.filter((a) => a.isActive).map((a) => a.id);
    startBackgroundSync(activeIds, true);
  }, []);

  if (!initialized) {
    return (
      <div className="flex h-screen items-center justify-center bg-bg-primary">
        <div className="flex flex-col items-center gap-4">
          <div className="relative w-10 h-10">
            <div className="absolute inset-0 rounded-full border-2 border-accent/20" />
            <div className="absolute inset-0 rounded-full border-2 border-transparent border-t-accent animate-spin" />
          </div>
          <span className="text-xs text-text-tertiary animate-pulse">
            {t("settings:loadingInbox")}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen overflow-hidden text-text-primary">
      <OfflineBanner />
      {/* Animated gradient blobs for glassmorphism effect */}
      <div className="animated-bg" aria-hidden="true">
        <div className="blob" />
        <div className="blob" />
        <div className="blob" />
        <div className="blob" />
        <div className="blob" />
      </div>
      <TitleBar />
      <div className="flex flex-1 min-w-0 overflow-hidden">
        <DndProvider>
          <ErrorBoundary name="Sidebar">
            <Sidebar
              collapsed={sidebarCollapsed}
              onAddAccount={(): void => setShowAddAccount(true)}
            />
          </ErrorBoundary>
          <Outlet />
        </DndProvider>
      </div>

      {/* Sync status bar */}
      {showSyncStatusBar && syncStatus != null && (
        <div
          className={`fixed bottom-0 right-0 glass-panel text-white text-xs px-4 py-1.5 text-center z-40 transition-all duration-200 ${syncStatus.startsWith("Sync failed") ? "bg-danger/90" : "bg-accent/90"}`}
          style={{ left: sidebarCollapsed ? "4rem" : "15rem" }}
        >
          {syncStatus}
        </div>
      )}

      {showAddAccount === true && (
        <AddAccount
          onClose={(): void => setShowAddAccount(false)}
          onSuccess={handleAddAccountSuccess}
        />
      )}

      <ErrorBoundary name="Composer">
        <Composer />
      </ErrorBoundary>
      <UndoSendToast />
      <UpdateToast />
      <ErrorBoundary name="CommandPalette">
        <CommandPalette
          isOpen={showCommandPalette}
          onClose={(): void => setShowCommandPalette(false)}
        />
      </ErrorBoundary>
      <ShortcutsHelp
        isOpen={showShortcutsHelp}
        onClose={(): void => setShowShortcutsHelp(false)}
      />
      <ErrorBoundary name="AskInbox">
        <AskInbox
          isOpen={showAskInbox}
          onClose={(): void => setShowAskInbox(false)}
        />
      </ErrorBoundary>
      <ContextMenuPortal />
      <MoveToFolderDialog
        isOpen={moveToFolderState.open}
        threadIds={moveToFolderState.threadIds}
        onClose={(): void =>
          setMoveToFolderState({ open: false, threadIds: [] })
        }
      />
    </div>
  );
}
