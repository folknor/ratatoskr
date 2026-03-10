/**
 * Core composer facade — re-exports every composer-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

import { invoke } from "@tauri-apps/api/core";

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
 */
export async function updateScheduledEmailAttachments(
  accountId: string,
  attachmentData: string,
): Promise<void> {
  await invoke("db_update_scheduled_email_attachments", {
    accountId,
    attachmentData,
  });
}
