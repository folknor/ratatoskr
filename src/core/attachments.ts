/**
 * Core attachments facade — re-exports every attachment-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

// Attachment DB queries
export {
  type AttachmentSender,
  type AttachmentWithContext,
  type DbAttachment,
  getAttachmentSenders,
  getAttachmentsForAccount,
  getAttachmentsForMessage,
} from "@/services/db/attachments";

// Email provider (for downloading attachments)
export { getEmailProvider } from "@/services/email/providerFactory";
