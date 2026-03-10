import { invoke } from "@tauri-apps/api/core";

export interface LocalDraft {
  id: string;
  account_id: string;
  to_addresses: string | null;
  cc_addresses: string | null;
  bcc_addresses: string | null;
  subject: string | null;
  body_html: string | null;
  reply_to_message_id: string | null;
  thread_id: string | null;
  from_email: string | null;
  signature_id: string | null;
  remote_draft_id: string | null;
  attachments: string | null;
  created_at: number;
  updated_at: number;
  sync_status: string;
}

export async function upsertLocalDraft(draft: {
  id: string;
  account_id: string;
  to_addresses?: string | null;
  cc_addresses?: string | null;
  bcc_addresses?: string | null;
  subject?: string | null;
  body_html?: string | null;
  reply_to_message_id?: string | null;
  thread_id?: string | null;
  from_email?: string | null;
  signature_id?: string | null;
  remote_draft_id?: string | null;
  attachments?: string | null;
}): Promise<void> {
  return invoke<void>("db_save_local_draft", {
    id: draft.id,
    accountId: draft.account_id,
    toAddresses: draft.to_addresses ?? null,
    ccAddresses: draft.cc_addresses ?? null,
    bccAddresses: draft.bcc_addresses ?? null,
    subject: draft.subject ?? null,
    bodyHtml: draft.body_html ?? null,
    replyToMessageId: draft.reply_to_message_id ?? null,
    threadId: draft.thread_id ?? null,
    fromEmail: draft.from_email ?? null,
    signatureId: draft.signature_id ?? null,
    remoteDraftId: draft.remote_draft_id ?? null,
    attachments: draft.attachments ?? null,
  });
}

export async function getLocalDraft(id: string): Promise<LocalDraft | null> {
  return invoke<LocalDraft | null>("db_get_local_draft", { id });
}

export async function getUnsyncedDrafts(
  accountId: string,
): Promise<LocalDraft[]> {
  return invoke<LocalDraft[]>("db_get_unsynced_drafts", { accountId });
}

export async function markDraftSynced(
  id: string,
  remoteDraftId: string,
): Promise<void> {
  return invoke<void>("db_mark_draft_synced", { id, remoteDraftId });
}

export async function deleteLocalDraft(id: string): Promise<void> {
  return invoke<void>("db_delete_local_draft", { id });
}
