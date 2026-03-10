import { invoke } from "@tauri-apps/api/core";

export interface DbThread {
  id: string;
  account_id: string;
  subject: string | null;
  snippet: string | null;
  last_message_at: number | null;
  message_count: number;
  is_read: number | boolean;
  is_starred: number | boolean;
  is_important: number | boolean;
  has_attachments: number | boolean;
  is_snoozed: number | boolean;
  snooze_until: number | null;
  is_pinned: number | boolean;
  is_muted: number | boolean;
  from_name: string | null;
  from_address: string | null;
}

export async function getThreadsForAccount(
  accountId: string,
  labelId?: string,
  limit: number = 50,
  offset: number = 0,
): Promise<DbThread[]> {
  return invoke<DbThread[]>("db_get_threads", {
    accountId,
    labelId,
    limit,
    offset,
  });
}

export async function getThreadsForCategory(
  accountId: string,
  category: string,
  limit: number = 50,
  offset: number = 0,
): Promise<DbThread[]> {
  return invoke<DbThread[]>("db_get_threads_for_category", {
    accountId,
    category,
    limit,
    offset,
  });
}

export async function upsertThread(thread: {
  id: string;
  accountId: string;
  subject: string | null;
  snippet: string | null;
  lastMessageAt: number | null;
  messageCount: number;
  isRead: boolean;
  isStarred: boolean;
  isImportant: boolean;
  hasAttachments: boolean;
}): Promise<void> {
  return invoke<void>("db_upsert_thread", {
    id: thread.id,
    accountId: thread.accountId,
    subject: thread.subject,
    snippet: thread.snippet,
    lastMessageAt: thread.lastMessageAt,
    messageCount: thread.messageCount,
    isRead: thread.isRead,
    isStarred: thread.isStarred,
    isImportant: thread.isImportant,
    hasAttachments: thread.hasAttachments,
  });
}

export async function setThreadLabels(
  accountId: string,
  threadId: string,
  labelIds: string[],
): Promise<void> {
  return invoke<void>("db_set_thread_labels", {
    accountId,
    threadId,
    labelIds,
  });
}

export async function getThreadLabelIds(
  accountId: string,
  threadId: string,
): Promise<string[]> {
  return invoke<string[]>("db_get_thread_label_ids", {
    accountId,
    threadId,
  });
}

export async function getThreadById(
  accountId: string,
  threadId: string,
): Promise<DbThread | undefined> {
  const row = await invoke<DbThread | null>("db_get_thread_by_id", {
    accountId,
    threadId,
  });
  return row ?? undefined;
}

export async function getThreadCountForAccount(
  accountId: string,
): Promise<number> {
  return invoke<number>("db_get_thread_count", { accountId });
}

export async function getUnreadInboxCount(): Promise<number> {
  return invoke<number>("db_get_unread_inbox_count");
}

export async function deleteThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_delete_thread", { accountId, threadId });
}

export async function deleteAllThreadsForAccount(
  accountId: string,
): Promise<void> {
  return invoke<void>("db_delete_all_threads_for_account", { accountId });
}

export async function pinThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_set_thread_pinned", {
    accountId,
    threadId,
    isPinned: true,
  });
}

export async function unpinThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_set_thread_pinned", {
    accountId,
    threadId,
    isPinned: false,
  });
}

export async function muteThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_set_thread_muted", {
    accountId,
    threadId,
    isMuted: true,
  });
}

export async function unmuteThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_set_thread_muted", {
    accountId,
    threadId,
    isMuted: false,
  });
}

export async function getMutedThreadIds(
  accountId: string,
): Promise<Set<string>> {
  const ids = await invoke<string[]>("db_get_muted_thread_ids", { accountId });
  return new Set(ids);
}
