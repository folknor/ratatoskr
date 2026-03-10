import { invoke } from "@tauri-apps/api/core";
import { deleteThread as deleteThreadFromDb } from "../db/threads";

interface GmailDraftStub {
  id: string;
  message: { id: string; threadId: string };
}

/**
 * Delete all drafts for a given thread via Rust Tauri commands, then remove the thread from local DB.
 * This is the correct way to delete drafts — using the Drafts API permanently removes them,
 * unlike modifyThread(["TRASH"]) which only trashes but leaves the DRAFT label intact.
 */
export async function deleteDraftsForThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  const drafts = await invoke<GmailDraftStub[]>("gmail_list_drafts", {
    accountId,
  });
  const threadDrafts = drafts.filter((d) => d.message?.threadId === threadId);
  for (const d of threadDrafts) {
    await invoke("gmail_delete_draft", { accountId, draftId: d.id });
  }
  await deleteThreadFromDb(accountId, threadId);
}
