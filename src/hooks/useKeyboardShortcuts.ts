import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useRef } from "react";
import { parseUnsubscribeUrl } from "@/components/email/MessageItem";
import {
  getActiveLabel,
  getSelectedThreadId,
  navigateBack,
  navigateToLabel,
  navigateToThread,
} from "@/router/navigate";
import {
  archiveThread,
  deleteThread as deleteThreadFromDb,
  deleteDraftsForThread,
  getGmailClient,
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
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { useContextMenuStore } from "@/stores/contextMenuStore";
import { useShortcutStore } from "@/stores/shortcutStore";
import { useThreadStore } from "@/stores/threadStore";
import { useUIStore } from "@/stores/uiStore";

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
            window.dispatchEvent(new Event("velo-toggle-command-palette"));
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
        const paneOff = useUIStore.getState().readingPanePosition === "hidden";
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
      if (useUIStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Primary" });
      }
      break;
    case "nav.goUpdates":
      if (useUIStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Updates" });
      }
      break;
    case "nav.goPromotions":
      if (useUIStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Promotions" });
      }
      break;
    case "nav.goSocial":
      if (useUIStore.getState().inboxViewMode === "split") {
        navigateToLabel("inbox", { category: "Social" });
      }
      break;
    case "nav.goNewsletters":
      if (useUIStore.getState().inboxViewMode === "split") {
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
        const replyMode = useUIStore.getState().defaultReplyMode;
        window.dispatchEvent(
          new CustomEvent("velo-inline-reply", { detail: { mode: replyMode } }),
        );
      }
      break;
    }
    case "action.replyAll":
      if (selectedId) {
        window.dispatchEvent(
          new CustomEvent("velo-inline-reply", {
            detail: { mode: "replyAll" },
          }),
        );
      }
      break;
    case "action.forward":
      if (selectedId) {
        window.dispatchEvent(
          new CustomEvent("velo-inline-reply", { detail: { mode: "forward" } }),
        );
      }
      break;
    case "action.archive": {
      const multiIds = useThreadStore.getState().selectedThreadIds;
      if (multiIds.size > 0 && activeAccountId) {
        const ids = [...multiIds];
        for (const id of ids) {
          await archiveThread(activeAccountId, id, []);
        }
      } else if (selectedId && activeAccountId) {
        await archiveThread(activeAccountId, selectedId, []);
      }
      break;
    }
    case "action.delete": {
      const deleteLabelCtx = getActiveLabel();
      const isTrashView = deleteLabelCtx === "trash";
      const isDraftsView = deleteLabelCtx === "drafts";
      const multiDeleteIds = useThreadStore.getState().selectedThreadIds;
      if (multiDeleteIds.size > 0 && activeAccountId) {
        const ids = [...multiDeleteIds];
        for (const id of ids) {
          if (isTrashView) {
            await permanentDeleteThread(activeAccountId, id, []);
            await deleteThreadFromDb(activeAccountId, id);
          } else if (isDraftsView) {
            try {
              const client = await getGmailClient(activeAccountId);
              await deleteDraftsForThread(client, activeAccountId, id);
              useThreadStore.getState().removeThread(id);
            } catch (err) {
              console.error("Draft delete failed:", err);
            }
          } else {
            await trashThread(activeAccountId, id, []);
          }
        }
      } else if (selectedId && activeAccountId) {
        if (isTrashView) {
          await permanentDeleteThread(activeAccountId, selectedId, []);
          await deleteThreadFromDb(activeAccountId, selectedId);
        } else if (isDraftsView) {
          try {
            const client = await getGmailClient(activeAccountId);
            await deleteDraftsForThread(client, activeAccountId, selectedId);
            useThreadStore.getState().removeThread(selectedId);
          } catch (err) {
            console.error("Draft delete failed:", err);
          }
        } else {
          await trashThread(activeAccountId, selectedId, []);
        }
      }
      break;
    }
    case "action.star": {
      if (selectedId && activeAccountId) {
        const thread = threads.find((t) => t.id === selectedId);
        if (thread) {
          await starThread(activeAccountId, selectedId, [], !thread.isStarred);
        }
      }
      break;
    }
    case "action.spam": {
      const isSpamView = getActiveLabel() === "spam";
      const multiSpamIds = useThreadStore.getState().selectedThreadIds;
      if (multiSpamIds.size > 0 && activeAccountId) {
        const ids = [...multiSpamIds];
        for (const id of ids) {
          await spamThread(activeAccountId, id, [], !isSpamView);
        }
      } else if (selectedId && activeAccountId) {
        await spamThread(activeAccountId, selectedId, [], !isSpamView);
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
            const url = parseUnsubscribeUrl(unsubMsg.list_unsubscribe);
            if (url) {
              await openUrl(url);
              await archiveThread(activeAccountId, selectedId, []);
            }
          }
        } catch (err) {
          console.error("Unsubscribe failed:", err);
        }
      }
      break;
    }
    case "action.mute": {
      const multiMuteIds = useThreadStore.getState().selectedThreadIds;
      if (multiMuteIds.size > 0 && activeAccountId) {
        const ids = [...multiMuteIds];
        for (const id of ids) {
          const t = threads.find((thread) => thread.id === id);
          if (t?.isMuted) {
            await unmuteThread(activeAccountId, id);
          } else {
            await muteThread(activeAccountId, id, []);
          }
        }
      } else if (selectedId && activeAccountId) {
        const thread = threads.find((t) => t.id === selectedId);
        if (thread) {
          if (thread.isMuted) {
            await unmuteThread(activeAccountId, selectedId);
          } else {
            await muteThread(activeAccountId, selectedId, []);
          }
        }
      }
      break;
    }
    case "action.createTaskFromEmail": {
      if (selectedId) {
        window.dispatchEvent(
          new CustomEvent("velo-extract-task", {
            detail: { threadId: selectedId },
          }),
        );
      }
      break;
    }
    case "action.moveToFolder": {
      const multiMoveIds = useThreadStore.getState().selectedThreadIds;
      const moveThreadIds =
        multiMoveIds.size > 0
          ? [...multiMoveIds]
          : selectedId
            ? [selectedId]
            : [];
      if (moveThreadIds.length > 0) {
        window.dispatchEvent(
          new CustomEvent("velo-move-to-folder", {
            detail: { threadIds: moveThreadIds },
          }),
        );
      }
      break;
    }
    case "app.commandPalette":
      window.dispatchEvent(new Event("velo-toggle-command-palette"));
      break;
    case "app.toggleSidebar":
      useUIStore.getState().toggleSidebar();
      break;
    case "app.askInbox":
      window.dispatchEvent(new Event("velo-toggle-ask-inbox"));
      break;
    case "app.help":
      window.dispatchEvent(new Event("velo-toggle-shortcuts-help"));
      break;
    case "app.syncFolder": {
      if (activeAccountId) {
        const currentLabel = getActiveLabel();
        useUIStore.getState().setSyncingFolder(currentLabel);
        triggerSync([activeAccountId]);
      }
      break;
    }
  }
}
