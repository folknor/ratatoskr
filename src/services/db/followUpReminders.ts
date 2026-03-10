import { invoke } from "@tauri-apps/api/core";

export interface DbFollowUpReminder {
  id: string;
  account_id: string;
  thread_id: string;
  message_id: string;
  remind_at: number;
  status: string;
  created_at: number;
}

export async function insertFollowUpReminder(
  accountId: string,
  threadId: string,
  messageId: string,
  remindAt: number,
): Promise<void> {
  const id = crypto.randomUUID();
  await invoke("db_insert_follow_up_reminder", {
    id,
    accountId,
    threadId,
    messageId,
    remindAt,
  });
}

export async function getFollowUpForThread(
  accountId: string,
  threadId: string,
): Promise<DbFollowUpReminder | null> {
  return invoke<DbFollowUpReminder | null>("db_get_follow_up_for_thread", {
    accountId,
    threadId,
  });
}

export async function cancelFollowUpForThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  await invoke("db_cancel_follow_up_for_thread", {
    accountId,
    threadId,
  });
}

export async function getActiveFollowUpThreadIds(
  accountId: string,
  threadIds: string[],
): Promise<Set<string>> {
  if (threadIds.length === 0) return new Set();
  const ids = await invoke<string[]>("db_get_active_follow_up_thread_ids", {
    accountId,
    threadIds,
  });
  return new Set(ids);
}
