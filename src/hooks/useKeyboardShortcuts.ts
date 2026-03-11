import { useEffect, useRef } from "react";
import {
  archiveThread,
  deleteDraftsForThread,
  deleteThread as deleteThreadFromDb,
  muteThread,
  permanentDeleteThread,
  pinThread,
  spamThread,
  starThread,
  trashThread,
  triggerSync,
  unmuteThread,
  unpinThread,
} from "@/core/mutations";
import { getMessagesForThread } from "@/core/queries";
import {
  getActiveLabel,
  getSelectedThreadId,
  navigateBack,
  navigateToLabel,
  navigateToThread,
} from "@/router/navigate";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { useContextMenuStore } from "@/stores/contextMenuStore";
import { useShortcutStore } from "@/stores/shortcutStore";
import { useSyncStateStore } from "@/stores/syncStateStore";
import { useThreadStore } from "@/stores/threadStore";
import { useUILayoutStore } from "@/stores/uiLayoutStore";
import { useUIPreferencesStore } from "@/stores/uiPreferencesStore";
import { resolveKeyboardTargets } from "@/utils/multiSelectTargets";

/**
 * Parse a key binding string and check if it matches a keyboard event.
 * Supports formats like: "j", "#", "Ctrl+K", "Ctrl+Shift+E", "Ctrl+Enter"
 */
function matchesKey(binding: string, e: KeyboardEvent): boolean {
  const parts = binding.split("+");
  const key = parts[parts.length - 1] ?? "";
  const needsCtrl = parts.some((p) => p === "Ctrl" || p === "Cmd");
  const needsShift = parts.some((p) => p === "Shift");
  const needsAlt = parts.some((p) => p === "Alt");

  const ctrlMatch = needsCtrl
    ? e.ctrlKey || e.metaKey
    : !(e.ctrlKey || e.metaKey);
  const shiftMatch = needsShift ? e.shiftKey : !e.shiftKey;
  const altMatch = needsAlt ? e.altKey : !e.altKey;

  // For single character keys, compare case-insensitively
  const keyMatch =
    key.length === 1
      ? e.key === key ||
        e.key === key.toLowerCase() ||
        e.key === key.toUpperCase()
      : e.key === key;

  return ctrlMatch && shiftMatch && altMatch && keyMatch;
}

/**
 * Build a reverse map: key binding -> action ID.
 * For "g then X" sequences, stores as "g then X" literally.
 */
function buildReverseMap(keyMap: Record<string, string>): {
  singleKey: Map<string, string>;
  twoKeySequences: Map<string, string>; // second key -> action ID (first key is always "g")
  ctrlCombos: Map<string, string>;
} {
  const singleKey = new Map<string, string>();
  const twoKeySequences = new Map<string, string>();
  const ctrlCombos = new Map<string, string>();

  for (const [id, keys] of Object.entries(keyMap)) {
    if (keys.includes(" then ")) {
      // Two-key sequence like "g then i"
      const secondKey = keys.split(" then ")[1]?.trim() ?? "";
      twoKeySequences.set(secondKey, id);
    } else if (
      keys.includes("+") &&
      (keys.includes("Ctrl") || keys.includes("Cmd"))
    ) {
      ctrlCombos.set(id, keys);
    } else {
      singleKey.set(keys, id);
    }
  }

  return { singleKey, twoKeySequences, ctrlCombos };
}

// Cached reverse map to avoid rebuilding on every keypress
let cachedKeyMap: Record<string, string> | null = null;
let cachedReverseMap: ReturnType<typeof buildReverseMap> | null = null;

function getCachedReverseMap(
  keyMap: Record<string, string>,
): ReturnType<typeof buildReverseMap> {
  if (cachedKeyMap === keyMap && cachedReverseMap) return cachedReverseMap;
  cachedKeyMap = keyMap;
  cachedReverseMap = buildReverseMap(keyMap);
  return cachedReverseMap;
}

/**
 * Global keyboard shortcuts handler (Superhuman-inspired).
 * Uses customizable key bindings from the shortcut store.
 */
export function useKeyboardShortcuts(): void {
  const pendingKeyRef = useRef<string | null>(null);
  const pendingTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const handleKeyDown = async (e: KeyboardEvent): Promise<void> => {
      // Close context menu on Escape before any other handling
      if (e.key === "Escape" && useContextMenuStore.getState().menuType) {
        e.preventDefault();
        useContextMenuStore.getState().closeMenu();
        return;
      }

      const target = e.target as HTMLElement;
      const isInputFocused =
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable;

      const keyMap = useShortcutStore.getState().keyMap;
      const { singleKey, twoKeySequences, ctrlCombos } =
        getCachedReverseMap(keyMap);

      // Ctrl/Cmd shortcuts
      if (e.ctrlKey || e.metaKey) {
        // Let native text-editing shortcuts work in inputs (select all, copy, cut, paste, undo, redo)
        if (
          isInputFocused &&
          ["a", "c", "x", "v", "z"].includes(e.key.toLowerCase())
        )
          return;

        for (const [ctrlActionId, binding] of ctrlCombos) {
          if (matchesKey(binding, e)) {
            e.preventDefault();
            await executeAction(ctrlActionId);
            return;
          }
        }
        // Ctrl+K for command palette (also check binding)
        if (e.key === "k" && !e.shiftKey) {
          const paletteBinding = keyMap["app.commandPalette"];
          if (
            paletteBinding === "Ctrl+K" ||
            paletteBinding === "/" ||
            !paletteBinding
          ) {
            e.preventDefault();
            window.dispatchEvent(new Event("ratatoskr-toggle-command-palette"));
            return;
          }
        }
        if (e.key === "Enter") {
          // Send email shortcut handled by composer
          return;
        }
        return;
      }

      // F5 sync works even when input is focused
      if (e.key === "F5") {
        e.preventDefault();
        const syncActionId = singleKey.get("F5");
        if (syncActionId) {
          await executeAction(syncActionId);
        }
        return;
      }

      // Don't process single-key shortcuts when typing in inputs
      if (isInputFocused) return;

      const key = e.key;

      // Handle two-key sequences (pending "g" key)
      if (pendingKeyRef.current === "g") {
        pendingKeyRef.current = null;
        if (pendingTimerRef.current) {
          clearTimeout(pendingTimerRef.current);
          pendingTimerRef.current = null;
        }
        const seqActionId = twoKeySequences.get(key);
        if (seqActionId) {
          e.preventDefault();
          void executeAction(seqActionId);
          return;
        }
      }

      // Check if "g" starts a two-key sequence
      if (key === "g" && twoKeySequences.size > 0) {
        pendingKeyRef.current = "g";
        pendingTimerRef.current = setTimeout(() => {
          pendingKeyRef.current = null;
        }, 1000);
        return;
      }

      // Arrow keys navigate the thread list when no thread is open full-screen
      // (In split-pane mode or list-only view, arrows move between threads)
      if (key === "ArrowDown" || key === "ArrowUp") {
        const selectedId = getSelectedThreadId();
        const paneOff =
          useUILayoutStore.getState().readingPanePosition === "hidden";
        // Only handle here if no thread is open in full-screen mode
        // (when pane is off and a thread is selected, ThreadView handles arrows for message nav)
        if (!(paneOff && selectedId)) {
          e.preventDefault();
          await executeAction(key === "ArrowDown" ? "nav.next" : "nav.prev");
          return;
        }
      }

      // Single key shortcuts
      let actionId = singleKey.get(key);
      // Delete and Backspace always trigger delete action
      if (!actionId && (key === "Delete" || key === "Backspace")) {
        actionId = "action.delete";
      }
      if (actionId) {
        e.preventDefault();
        await executeAction(actionId);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return (): void => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}

async function executeAction(actionId: string): Promise<void> {
  const threads = useThreadStore.getState().threads;
  const selectedId = getSelectedThreadId();
  const currentIdx = threads.findIndex((t) => t.id === selectedId);
  const activeAccountId = useAccountStore.getState().activeAccountId;

  switch (actionId) {
    case "nav.next": {
      const nextIdx = Math.min(currentIdx + 1, threads.length - 1);
      if (threads[nextIdx]) {
        navigateToThread(threads[nextIdx].id);
      }
      break;
    }
    case "nav.prev": {
      const prevIdx = Math.max(currentIdx - 1, 0);
      if (threads[prevIdx]) {
        navigateToThread(threads[prevIdx].id);
      }
      break;
    }
    case "nav.open": {
      if (!selectedId && threads[0]) {
        navigateToThread(threads[0].id);
      }
      break;
    }
    case "nav.goInbox":
      navigateToLabel("inbox");
      break;
    case "nav.goStarred":
      navigateToLabel("starred");
      break;
    case "nav.goSent":
      navigateToLabel("sent");
      break;
    case "nav.goDrafts":
      navigateToLabel("drafts");
      break;
    case "nav.goPrimary":
      if (useUIPreferencesStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Primary" });
      }
      break;
    case "nav.goUpdates":
      if (useUIPreferencesStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Updates" });
      }
      break;
    case "nav.goPromotions":
      if (useUIPreferencesStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Promotions" });
      }
      break;
    case "nav.goSocial":
      if (useUIPreferencesStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Social" });
      }
      break;
    case "nav.goNewsletters":
      if (useUIPreferencesStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Newsletters" });
      }
      break;
    case "nav.goTasks":
      navigateToLabel("tasks");
      break;
    case "nav.goAttachments":
      navigateToLabel("attachments");
      break;
    case "nav.escape": {
      if (useComposerStore.getState().isOpen) {
        useComposerStore.getState().closeComposer();
      } else if (useThreadStore.getState().selectedThreadIds.size > 0) {
        useThreadStore.getState().clearMultiSelect();
      } else if (selectedId) {
        navigateBack();
      }
      break;
    }
    case "action.compose":
      useComposerStore.getState().openComposer();
      break;
    case "action.reply": {
      if (selectedId) {
        const replyMode = useUIPreferencesStore.getState().defaultReplyMode;
        window.dispatchEvent(
          new CustomEvent("ratatoskr-inline-reply", {
            detail: { mode: replyMode },
          }),
        );
      }
      break;
    }
    case "action.replyAll":
      if (selectedId) {
        window.dispatchEvent(
          new CustomEvent("ratatoskr-inline-reply", {
            detail: { mode: "replyAll" },
          }),
        );
      }
      break;
    case "action.forward":
      if (selectedId) {
        window.dispatchEvent(
          new CustomEvent("ratatoskr-inline-reply", {
            detail: { mode: "forward" },
          }),
        );
      }
      break;
    case "action.archive": {
      const archiveIds = resolveKeyboardTargets(
        useThreadStore.getState().selectedThreadIds,
        selectedId,
      );
      if (archiveIds.length > 0 && activeAccountId) {
        for (const id of archiveIds) {
          await archiveThread(activeAccountId, id);
        }
      }
      break;
    }
    case "action.delete": {
      const deleteLabelCtx = getActiveLabel();
      const isTrashView = deleteLabelCtx === "trash";
      const isDraftsView = deleteLabelCtx === "drafts";
      const deleteIds = resolveKeyboardTargets(
        useThreadStore.getState().selectedThreadIds,
        selectedId,
      );
      if (deleteIds.length > 0 && activeAccountId) {
        for (const id of deleteIds) {
          if (isTrashView) {
            await permanentDeleteThread(activeAccountId, id);
            await deleteThreadFromDb(activeAccountId, id);
          } else if (isDraftsView) {
            try {
              await deleteDraftsForThread(activeAccountId, id);
              useThreadStore.getState().removeThread(id);
            } catch (err) {
              console.error("Draft delete failed:", err);
            }
          } else {
            await trashThread(activeAccountId, id);
          }
        }
      }
      break;
    }
    case "action.star": {
      if (selectedId && activeAccountId) {
        const thread = threads.find((t) => t.id === selectedId);
        if (thread) {
          await starThread(activeAccountId, selectedId, !thread.isStarred);
        }
      }
      break;
    }
    case "action.spam": {
      const isSpamView = getActiveLabel() === "spam";
      const spamIds = resolveKeyboardTargets(
        useThreadStore.getState().selectedThreadIds,
        selectedId,
      );
      if (spamIds.length > 0 && activeAccountId) {
        for (const id of spamIds) {
          await spamThread(activeAccountId, id, !isSpamView);
        }
      }
      break;
    }
    case "action.pin": {
      if (selectedId && activeAccountId) {
        const thread = threads.find((t) => t.id === selectedId);
        if (thread) {
          if (thread.isPinned) {
            await unpinThread(activeAccountId, selectedId);
          } else {
            await pinThread(activeAccountId, selectedId);
          }
        }
      }
      break;
    }
    case "action.selectAll": {
      useThreadStore.getState().selectAll();
      break;
    }
    case "action.selectFromHere": {
      useThreadStore.getState().selectAllFromHere();
      break;
    }
    case "action.unsubscribe": {
      if (selectedId && activeAccountId) {
        try {
          const msgs = await getMessagesForThread(activeAccountId, selectedId);
          const unsubMsg = msgs.find((m) => m.list_unsubscribe);
          if (unsubMsg?.list_unsubscribe) {
            const { executeUnsubscribe } = await import("@/core/mutations");
            await executeUnsubscribe(
              activeAccountId,
              selectedId,
              unsubMsg.from_address ?? "",
              unsubMsg.from_name ?? null,
              unsubMsg.list_unsubscribe,
              unsubMsg.list_unsubscribe_post ?? null,
            );
            await archiveThread(activeAccountId, selectedId);
          }
        } catch (err) {
          console.error("Unsubscribe failed:", err);
        }
      }
      break;
    }
    case "action.mute": {
      const muteIds = resolveKeyboardTargets(
        useThreadStore.getState().selectedThreadIds,
        selectedId,
      );
      if (muteIds.length > 0 && activeAccountId) {
        for (const id of muteIds) {
          const t = threads.find((thread) => thread.id === id);
          if (t?.isMuted) {
            await unmuteThread(activeAccountId, id);
          } else {
            await muteThread(activeAccountId, id);
          }
        }
      }
      break;
    }
    case "action.createTaskFromEmail": {
      if (selectedId) {
        window.dispatchEvent(
          new CustomEvent("ratatoskr-extract-task", {
            detail: { threadId: selectedId },
          }),
        );
      }
      break;
    }
    case "action.moveToFolder": {
      const moveThreadIds = resolveKeyboardTargets(
        useThreadStore.getState().selectedThreadIds,
        selectedId,
      );
      if (moveThreadIds.length > 0) {
        window.dispatchEvent(
          new CustomEvent("ratatoskr-move-to-folder", {
            detail: { threadIds: moveThreadIds },
          }),
        );
      }
      break;
    }
    case "app.commandPalette":
      window.dispatchEvent(new Event("ratatoskr-toggle-command-palette"));
      break;
    case "app.toggleSidebar":
      useUILayoutStore.getState().toggleSidebar();
      break;
    case "app.askInbox":
      window.dispatchEvent(new Event("ratatoskr-toggle-ask-inbox"));
      break;
    case "app.help":
      window.dispatchEvent(new Event("ratatoskr-toggle-shortcuts-help"));
      break;
    case "app.syncFolder": {
      if (activeAccountId) {
        const currentLabel = getActiveLabel();
        useSyncStateStore.getState().setSyncingFolder(currentLabel);
        triggerSync([activeAccountId]);
      }
      break;
    }
  }
}
