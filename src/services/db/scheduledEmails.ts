import { getCurrentUnixTimestamp } from "@/utils/timestamp";
import { invoke } from "@tauri-apps/api/core";

export interface DbScheduledEmail {
  id: string;
  account_id: string;
  to_addresses: string;
  cc_addresses: string | null;
  bcc_addresses: string | null;
  subject: string | null;
  body_html: string;
  reply_to_message_id: string | null;
  thread_id: string | null;
  scheduled_at: number;
  signature_id: string | null;
  attachment_paths: string | null;
  status: string;
  created_at: number;
}

export async function getPendingScheduledEmails(): Promise<DbScheduledEmail[]> {
  const now = getCurrentUnixTimestamp();
  return invoke<DbScheduledEmail[]>("db_get_pending_scheduled_emails", {
    now,
  });
}

export async function getScheduledEmailsForAccount(
  accountId: string,
): Promise<DbScheduledEmail[]> {
  return invoke<DbScheduledEmail[]>(
    "db_get_scheduled_emails_for_account",
    { accountId },
  );
}

export async function insertScheduledEmail(email: {
  accountId: string;
  toAddresses: string;
  ccAddresses: string | null;
  bccAddresses: string | null;
  subject: string | null;
  bodyHtml: string;
  replyToMessageId: string | null;
  threadId: string | null;
  scheduledAt: number;
  signatureId: string | null;
}): Promise<string> {
  return invoke<string>("db_insert_scheduled_email", {
    accountId: email.accountId,
    toAddresses: email.toAddresses,
    ccAddresses: email.ccAddresses,
    bccAddresses: email.bccAddresses,
    subject: email.subject,
    bodyHtml: email.bodyHtml,
    replyToMessageId: email.replyToMessageId,
    threadId: email.threadId,
    scheduledAt: email.scheduledAt,
    signatureId: email.signatureId,
  });
}

export async function updateScheduledEmailStatus(
  id: string,
  status: "pending" | "sending" | "sent" | "failed" | "cancelled",
): Promise<void> {
  return invoke<void>("db_update_scheduled_email_status", { id, status });
}

export async function deleteScheduledEmail(id: string): Promise<void> {
  return invoke<void>("db_delete_scheduled_email", { id });
}
