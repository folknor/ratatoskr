import {
  Archive,
  Ban,
  Clock,
  ExternalLink,
  FolderInput,
  Forward,
  Layers,
  Mail,
  MailOpen,
  Pin,
  Reply,
  ReplyAll,
  Star,
  Tag,
  Trash2,
  VolumeX,
  Zap,
} from "lucide-react";
import type React from "react";
import { useEffect, useState } from "react";
import { getActiveLabel } from "@/router/navigate";
import {
  addThreadLabel,
  archiveThread,
  deleteThread as deleteThreadFromDb,
  deleteDraftsForThread,
  executeQuickStep,
  markThreadRead,
  muteThread,
  permanentDeleteThread,
  pinThread,
  removeThreadLabel,
  setThreadCategory,
  spamThread,
  starThread,
  trashThread,
  unmuteThread,
  unpinThread,
} from "@/core/mutations";
import {
  getMessagesForThread,
  type DbQuickStep,
  getEnabledQuickStepsForAccount,
  ALL_CATEGORIES,
  type QuickStep,
  type QuickStepAction,
} from "@/core/queries";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { useLabelStore } from "@/stores/labelStore";
import { useThreadStore } from "@/stores/threadStore";
import { buildQuote, buildForwardQuote } from "@/utils/emailQuoteBuilders";
import { resolveContextMenuTargets } from "@/utils/multiSelectTargets";
import { ContextMenu, type ContextMenuItem } from "../ContextMenu";
import type { ThreadMenuProps } from "./types";

export function ThreadContextMenu({
  position,
  data,
  onClose,
  onSnooze,
}: ThreadMenuProps): React.ReactNode {
  const threadId = data["threadId"] as string;
  const threads = useThreadStore((s) => s.threads);
  const selectedThreadIds = useThreadStore((s) => s.selectedThreadIds);
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const activeLabel = getActiveLabel();
  const labels = useLabelStore((s) => s.labels);
  const openComposer = useComposerStore((s) => s.openComposer);
  const [quickSteps, setQuickSteps] = useState<DbQuickStep[]>([]);

  useEffect(() => {
    if (!activeAccountId) return;
    getEnabledQuickStepsForAccount(activeAccountId)
      .then(setQuickSteps)
      .catch(() => {
        // quick_steps table may not exist yet before migration
      });
  }, [activeAccountId]);

  // Determine target threads: if right-clicked thread is in multi-select, use all selected; otherwise just this one
  const { targetIds, isMulti } = resolveContextMenuTargets(
    threadId,
    selectedThreadIds,
  );

  const thread = threads.find((t) => t.id === threadId);
  if (!(thread && activeAccountId)) {
    return <ContextMenu items={[]} position={position} onClose={onClose} />;
  }

  const isTrashView = activeLabel === "trash";
  const isDraftsView = activeLabel === "drafts";
  const isSpamView = activeLabel === "spam";

  // For single thread: show current state. For multi: be generic
  const isRead = isMulti ? true : thread.isRead;
  const isStarred = isMulti ? false : thread.isStarred;
  const isPinned = isMulti ? false : thread.isPinned;
  const isMuted = isMulti ? false : thread.isMuted;

  const handleReply = async (): Promise<void> => {
    const messages = await getMessagesForThread(activeAccountId, thread.id);
    const lastMessage = messages[messages.length - 1];
    if (!lastMessage) return;
    const replyTo = lastMessage.reply_to ?? lastMessage.from_address;
    openComposer({
      mode: "reply",
      to: replyTo ? [replyTo] : [],
      subject: `Re: ${lastMessage.subject ?? ""}`,
      bodyHtml: buildQuote(lastMessage),
      threadId: lastMessage.thread_id,
      inReplyToMessageId: lastMessage.id,
    });
  };

  const handleReplyAll = async (): Promise<void> => {
    const messages = await getMessagesForThread(activeAccountId, thread.id);
    const lastMessage = messages[messages.length - 1];
    if (!lastMessage) return;
    const replyTo = lastMessage.reply_to ?? lastMessage.from_address;
    const allRecipients = new Set<string>();
    if (replyTo) allRecipients.add(replyTo);
    if (lastMessage.to_addresses) {
      for (const a of lastMessage.to_addresses.split(",")) {
        allRecipients.add(a.trim());
      }
    }
    const ccList: string[] = [];
    if (lastMessage.cc_addresses) {
      for (const a of lastMessage.cc_addresses.split(",")) {
        ccList.push(a.trim());
      }
    }
    openComposer({
      mode: "replyAll",
      to: Array.from(allRecipients),
      cc: ccList,
      subject: `Re: ${lastMessage.subject ?? ""}`,
      bodyHtml: buildQuote(lastMessage),
      threadId: lastMessage.thread_id,
      inReplyToMessageId: lastMessage.id,
    });
  };

  const handleForward = async (): Promise<void> => {
    const messages = await getMessagesForThread(activeAccountId, thread.id);
    const lastMessage = messages[messages.length - 1];
    if (!lastMessage) return;
    openComposer({
      mode: "forward",
      to: [],
      subject: `Fwd: ${lastMessage.subject ?? ""}`,
      bodyHtml: buildForwardQuote(lastMessage),
      threadId: lastMessage.thread_id,
      inReplyToMessageId: lastMessage.id,
    });
  };

  const handleArchive = async (): Promise<void> => {
    for (const id of targetIds) {
      await archiveThread(activeAccountId, id, []);
    }
  };

  const handleDelete = async (): Promise<void> => {
    for (const id of targetIds) {
      if (isTrashView) {
        await permanentDeleteThread(activeAccountId, id, []);
        await deleteThreadFromDb(activeAccountId, id);
      } else if (isDraftsView) {
        useThreadStore.getState().removeThread(id);
        try {
          await deleteDraftsForThread(activeAccountId, id);
        } catch (err) {
          console.error("Failed to delete drafts:", err);
        }
      } else {
        await trashThread(activeAccountId, id, []);
      }
    }
  };

  const batchToggle = async (
    action: (id: string, current: boolean) => Promise<unknown>,
    getState: (t: (typeof threads)[number]) => boolean,
  ): Promise<void> => {
    for (const id of targetIds) {
      const t = threads.find((th) => th.id === id);
      if (!t) continue;
      await action(id, getState(t));
    }
  };

  const handleToggleRead = (): Promise<void> =>
    batchToggle(
      (id, isCurrentlyRead) =>
        markThreadRead(activeAccountId, id, [], !isCurrentlyRead),
      (t) => t.isRead,
    );

  const handleToggleStar = (): Promise<void> =>
    batchToggle(
      (id, isCurrentlyStarred) =>
        starThread(activeAccountId, id, [], !isCurrentlyStarred),
      (t) => t.isStarred,
    );

  const handleTogglePin = (): Promise<void> =>
    batchToggle(
      (id, isCurrentlyPinned) =>
        isCurrentlyPinned
          ? unpinThread(activeAccountId, id)
          : pinThread(activeAccountId, id),
      (t) => t.isPinned,
    );

  const handleToggleMute = (): Promise<void> =>
    batchToggle(
      (id, isCurrentlyMuted) =>
        isCurrentlyMuted
          ? unmuteThread(activeAccountId, id)
          : muteThread(activeAccountId, id, []),
      (t) => t.isMuted,
    );

  const handleSpam = async (): Promise<void> => {
    for (const id of targetIds) {
      await spamThread(activeAccountId, id, [], !isSpamView);
    }
  };

  const handleSnooze = (): void => {
    onSnooze({ threadIds: [...targetIds], accountId: activeAccountId });
  };

  const handlePopOut = async (): Promise<void> => {
    try {
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const windowLabel = `thread-${thread.id.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
      const url = `index.html?thread=${encodeURIComponent(thread.id)}&account=${encodeURIComponent(thread.accountId)}`;
      const existing = await WebviewWindow.getByLabel(windowLabel);
      if (existing) {
        await existing.setFocus();
        return;
      }
      const win = new WebviewWindow(windowLabel, {
        url,
        title: thread.subject ?? "Thread",
        width: 800,
        height: 700,
        center: true,
        dragDropEnabled: false,
      });
      win.once("tauri://error", (e) => {
        console.error("Failed to create pop-out window:", e);
      });
    } catch (err) {
      console.error("Failed to open pop-out window:", err);
    }
  };

  const handleToggleLabel = async (labelId: string): Promise<void> => {
    for (const id of targetIds) {
      const t = useThreadStore.getState().threads.find((th) => th.id === id);
      if (!t) continue;
      const hasLabel = t.labelIds.includes(labelId);
      if (hasLabel) {
        await removeThreadLabel(activeAccountId, id, labelId);
        useThreadStore.getState().updateThread(id, {
          labelIds: t.labelIds.filter((l) => l !== labelId),
        });
      } else {
        await addThreadLabel(activeAccountId, id, labelId);
        useThreadStore.getState().updateThread(id, {
          labelIds: [...t.labelIds, labelId],
        });
      }
    }
  };

  // Build label submenu items
  const labelItems: ContextMenuItem[] = labels.map((label) => {
    // For single thread, show checkmark if label is applied
    const isApplied = !isMulti && thread.labelIds.includes(label.id);
    return {
      id: `label-${label.id}`,
      label: label.name,
      checked: isApplied,
      action: () => handleToggleLabel(label.id),
    };
  });

  const items: ContextMenuItem[] = [
    {
      id: "reply",
      label: "Reply",
      icon: Reply,
      shortcut: "r",
      disabled: isMulti,
      action: handleReply,
    },
    {
      id: "reply-all",
      label: "Reply All",
      icon: ReplyAll,
      shortcut: "a",
      disabled: isMulti,
      action: handleReplyAll,
    },
    {
      id: "forward",
      label: "Forward",
      icon: Forward,
      shortcut: "f",
      disabled: isMulti,
      action: handleForward,
    },
    { id: "sep-1", label: "", separator: true },
    {
      id: "archive",
      label: "Archive",
      icon: Archive,
      shortcut: "e",
      action: handleArchive,
    },
    {
      id: "delete",
      label: isTrashView ? "Delete Permanently" : "Delete",
      icon: Trash2,
      shortcut: "#",
      danger: isTrashView,
      action: handleDelete,
    },
    {
      id: "toggle-read",
      label: isRead ? "Mark as Unread" : "Mark as Read",
      icon: isRead ? Mail : MailOpen,
      action: handleToggleRead,
    },
    {
      id: "toggle-star",
      label: isStarred ? "Unstar" : "Star",
      icon: Star,
      shortcut: "s",
      action: handleToggleStar,
    },
    { id: "sep-2", label: "", separator: true },
    {
      id: "snooze",
      label: "Snooze...",
      icon: Clock,
      shortcut: "h",
      action: handleSnooze,
    },
    {
      id: "toggle-pin",
      label: isPinned ? "Unpin" : "Pin",
      icon: Pin,
      shortcut: "p",
      action: handleTogglePin,
    },
    {
      id: "toggle-mute",
      label: isMuted ? "Unmute" : "Mute",
      icon: VolumeX,
      shortcut: "m",
      action: handleToggleMute,
    },
    {
      id: "spam",
      label: isSpamView ? "Not Spam" : "Report Spam",
      icon: Ban,
      shortcut: "!",
      action: handleSpam,
    },
    { id: "sep-3", label: "", separator: true },
    ...(labelItems.length > 0
      ? [
          {
            id: "apply-label",
            label: "Apply Label",
            icon: Tag,
            children: labelItems,
          },
        ]
      : []),
    {
      id: "move-to-folder",
      label: "Move to Folder",
      icon: FolderInput,
      shortcut: "v",
      action: () => {
        window.dispatchEvent(
          new CustomEvent("ratatoskr-move-to-folder", {
            detail: { threadIds: [...targetIds] },
          }),
        );
      },
    },
    {
      id: "move-to-category",
      label: "Move to Category",
      icon: Layers,
      children: ALL_CATEGORIES.map((cat) => ({
        id: `cat-${cat}`,
        label: cat,
        action: async () => {
          for (const id of targetIds) {
            await setThreadCategory(activeAccountId, id, cat, true);
          }
          window.dispatchEvent(new Event("ratatoskr-sync-done"));
        },
      })),
    },
    ...(quickSteps.length > 0
      ? [
          { id: "sep-4", label: "", separator: true },
          {
            id: "quick-steps",
            label: "Quick Steps",
            icon: Zap,
            children: quickSteps.map((qs) => {
              let parsedActions: QuickStepAction[] = [];
              try {
                parsedActions = JSON.parse(
                  qs.actions_json,
                ) as QuickStepAction[];
              } catch {
                /* ignore */
              }
              return {
                id: `qs-${qs.id}`,
                label: qs.name,
                action: async () => {
                  const step: QuickStep = {
                    id: qs.id,
                    accountId: qs.account_id,
                    name: qs.name,
                    description: qs.description,
                    shortcut: qs.shortcut,
                    actions: parsedActions,
                    icon: qs.icon,
                    isEnabled: qs.is_enabled === 1,
                    continueOnError: qs.continue_on_error === 1,
                    sortOrder: qs.sort_order,
                    createdAt: qs.created_at,
                  };
                  await executeQuickStep(step, [...targetIds], activeAccountId);
                },
              };
            }),
          } as ContextMenuItem,
        ]
      : []),
    {
      id: "pop-out",
      label: "Open in New Window",
      icon: ExternalLink,
      disabled: isMulti,
      action: handlePopOut,
    },
  ];

  return <ContextMenu items={items} position={position} onClose={onClose} />;
}
