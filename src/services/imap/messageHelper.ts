import { invoke } from "@tauri-apps/api/core";

export interface ImapMessageInfo {
  uid: number;
  folder: string;
}

/**
 * Look up imap_uid and imap_folder from the messages DB table for the given message IDs.
 * Only returns entries where both imap_uid and imap_folder are non-null.
 */
export async function getImapUidsForMessages(
  accountId: string,
  messageIds: string[],
): Promise<Map<string, ImapMessageInfo>> {
  if (messageIds.length === 0) {
    return new Map();
  }

  const rows = await invoke<
    { id: string; imap_uid: number | null; imap_folder: string | null }[]
  >("db_get_imap_uids_for_messages", { accountId, messageIds });

  const result = new Map<string, ImapMessageInfo>();
  for (const row of rows) {
    if (row.imap_uid != null && row.imap_folder != null) {
      result.set(row.id, { uid: row.imap_uid, folder: row.imap_folder });
    }
  }
  return result;
}

/**
 * Group IMAP UIDs by their folder path.
 */
export function groupMessagesByFolder(
  messages: Map<string, ImapMessageInfo>,
): Map<string, number[]> {
  const grouped = new Map<string, number[]>();
  for (const { uid, folder } of messages.values()) {
    const existing = grouped.get(folder);
    if (existing) {
      existing.push(uid);
    } else {
      grouped.set(folder, [uid]);
    }
  }
  return grouped;
}

/**
 * Map from special-use flags to the expected label IDs in the DB.
 */
const SPECIAL_USE_TO_LABEL_ID: Record<string, string> = {
  "\\Trash": "TRASH",
  "\\Junk": "SPAM",
  "\\Sent": "SENT",
  "\\Drafts": "DRAFT",
  "\\Archive": "archive",
};

/**
 * Find the folder path for a special-use folder (e.g. \\Trash, \\Junk, \\Sent, \\Drafts, \\Archive).
 * Looks up the labels table using the imap_special_use column first, then falls back to label ID.
 */
export async function findSpecialFolder(
  accountId: string,
  specialUse: string,
): Promise<string | null> {
  const fallbackLabelId = SPECIAL_USE_TO_LABEL_ID[specialUse] ?? null;
  return invoke("db_find_special_folder", {
    accountId,
    specialUse,
    fallbackLabelId,
  });
}

/**
 * Map DB security values to ImapConfig/SmtpConfig security types.
 * DB stores 'ssl'/'starttls'/'none', but configs use 'tls'/'starttls'/'none'.
 */
export function securityToConfigType(
  dbSecurity: string,
): "tls" | "starttls" | "none" {
  switch (dbSecurity) {
    case "ssl":
      return "tls";
    case "starttls":
      return "starttls";
    case "none":
      return "none";
    default:
      return "tls";
  }
}

/**
 * Update the imap_folder column for messages after a move operation.
 */
export async function updateMessageImapFolder(
  accountId: string,
  messageIds: string[],
  newFolder: string,
): Promise<void> {
  if (messageIds.length === 0) return;
  await invoke("db_update_message_imap_folder", {
    accountId,
    messageIds,
    newFolder,
  });
}
