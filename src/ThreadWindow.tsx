import type React from "react";
import { useEffect, useState } from "react";
import { Composer } from "./components/composer/Composer";
import { UndoSendToast } from "./components/composer/UndoSendToast";
import { ThreadView } from "./components/email/ThreadView";
import type { ColorThemeId } from "./constants/themes";
import { COLOR_THEMES, getThemeById } from "./constants/themes";
import { listAccountBasicInfo } from "./services/accounts/basicInfo";
import { getSetting } from "./services/db/settings";
import { getThreadById, getThreadLabelIds } from "./services/db/threads";
import { initializeClients } from "./services/gmail/tokenManager";
import { useAccountStore } from "./stores/accountStore";
import type { Thread } from "./stores/threadStore";
import { useUIPreferencesStore } from "./stores/uiPreferencesStore";

export default function ThreadWindow(): React.ReactNode {
  const { setTheme, setFontScale, setColorTheme } = useUIPreferencesStore();
  const { setAccounts } = useAccountStore();
  const [thread, setThread] = useState<Thread | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const threadIdParam = params.get("thread");
    const accountIdParam = params.get("account");

    if (!(threadIdParam && accountIdParam)) {
      setError("Missing thread or account parameter");
      setLoading(false);
      return;
    }

    const threadId: string = threadIdParam;
    const accountId: string = accountIdParam;

    async function init(): Promise<void> {
      try {
        // Load persisted language
        const { loadPersistedLanguage } = await import("./i18n");
        await loadPersistedLanguage();

        // Restore theme
        const savedTheme = await getSetting("theme");
        if (
          savedTheme === "light" ||
          savedTheme === "dark" ||
          savedTheme === "system"
        ) {
          setTheme(savedTheme);
        }

        // Restore font scale
        const savedFontScale = await getSetting("font_size");
        if (
          savedFontScale === "small" ||
          savedFontScale === "default" ||
          savedFontScale === "large" ||
          savedFontScale === "xlarge"
        ) {
          setFontScale(savedFontScale);
        }

        // Restore color theme
        const savedColorTheme = await getSetting("color_theme");
        if (
          savedColorTheme &&
          COLOR_THEMES.some((t) => t.id === savedColorTheme)
        ) {
          setColorTheme(savedColorTheme as ColorThemeId);
        }

        // Load accounts into store
        const dbAccounts = await listAccountBasicInfo();
        const mapped = dbAccounts.map((a) => ({
          id: a.id,
          email: a.email,
          displayName: a.displayName,
          avatarUrl: a.avatarUrl,
          isActive: a.isActive,
          provider: a.provider,
        }));
        setAccounts(mapped);

        // Set active account to the thread's account (without persisting to settings)
        useAccountStore.setState({ activeAccountId: accountId });

        // Initialize Gmail clients
        await initializeClients();

        // Fetch thread
        const dbThread = await getThreadById(accountId, threadId);
        if (!dbThread) {
          setError("Thread not found");
          setLoading(false);
          return;
        }

        const labelIds = await getThreadLabelIds(accountId, threadId);
        setThread({
          id: dbThread.id,
          accountId: dbThread.account_id,
          subject: dbThread.subject,
          snippet: dbThread.snippet,
          lastMessageAt: dbThread.last_message_at ?? 0,
          messageCount: dbThread.message_count,
          isRead: Boolean(dbThread.is_read),
          isStarred: Boolean(dbThread.is_starred),
          isPinned: Boolean(dbThread.is_pinned),
          isMuted: Boolean(dbThread.is_muted),
          hasAttachments: Boolean(dbThread.has_attachments),
          labelIds,
          fromName: dbThread.from_name,
          fromAddress: dbThread.from_address,
        });
      } catch (err) {
        console.error("Failed to initialize thread window:", err);
        setError("Failed to load thread");
      }
      setLoading(false);
    }

    void init();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- store setters are stable references
  }, [setAccounts, setColorTheme, setFontScale, setTheme]);

  // Sync theme class to <html>
  const theme = useUIPreferencesStore((s) => s.theme);
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
        if (mq.matches) root.classList.add("dark");
        else root.classList.remove("dark");
      };
      apply();
      mq.addEventListener("change", apply);
      return (): void => mq.removeEventListener("change", apply);
    }
  }, [theme]);

  // Sync font-scale class to <html>
  const fontScale = useUIPreferencesStore((s) => s.fontScale);
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

  // Apply color theme CSS custom properties to <html>
  const colorTheme = useUIPreferencesStore((s) => s.colorTheme);
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

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center bg-bg-primary text-text-secondary">
        <span className="text-sm">Loading thread...</span>
      </div>
    );
  }

  if (error || !thread) {
    return (
      <div className="flex h-screen items-center justify-center bg-bg-primary text-text-secondary">
        <span className="text-sm">{error ?? "Thread not found"}</span>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-bg-primary text-text-primary">
      <ThreadView thread={thread} />
      <Composer />
      <UndoSendToast />
    </div>
  );
}
