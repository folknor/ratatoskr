/**
 * Core composer facade — re-exports every composer-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

import { getDb } from "@/services/db/connection";

// Draft auto-save
export { startAutoSave, stopAutoSave } from "@/services/composer/draftAutoSave";
// Re-export LocalDraft type for consumers
export type { LocalDraft } from "@/services/db/localDrafts";
// Scheduled emails
export { insertScheduledEmail } from "@/services/db/scheduledEmails";
// Send-as aliases
export {
  type DbSendAsAlias,
  deleteAlias,
  getAliasesForAccount,
  getDefaultAlias,
  mapDbAlias,
  type SendAsAlias,
  setDefaultAlias,
  upsertAlias,
} from "@/services/db/sendAsAliases";
// Signatures
export {
  type DbSignature,
  deleteSignature,
  getDefaultSignature,
  getSignaturesForAccount,
  insertSignature,
  updateSignature,
} from "@/services/db/signatures";
// Templates
export {
  type DbTemplate,
  deleteTemplate,
  getTemplatesForAccount,
  insertTemplate,
  updateTemplate,
} from "@/services/db/templates";

/**
 * Look up a scheduled email's attachment data and update it.
 * Wraps the direct getDb() + SQL call previously in Composer.tsx.
 */
export async function updateScheduledEmailAttachments(
  accountId: string,
  attachmentData: string,
): Promise<void> {
  const db = await getDb();
  const rows = await db.select<{ id: string }[]>(
    "SELECT id FROM scheduled_emails WHERE account_id = $1 ORDER BY created_at DESC LIMIT 1",
    [accountId],
  );
  if (rows[0]) {
    await db.execute(
      "UPDATE scheduled_emails SET attachment_paths = $1 WHERE id = $2",
      [attachmentData, rows[0].id],
    );
  }
}
