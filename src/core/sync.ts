/**
 * Core sync facade — re-exports sync-related functions used by UI components.
 * UI code should import from here instead of reaching into @/services/gmail/* directly.
 */

// Sync triggers
export {
  forceFullSync,
  resyncAccount,
  triggerSync,
} from "@/services/gmail/syncManager";

// Gmail client access
export {
  getGmailClient,
  reauthorizeAccount,
  removeClient,
} from "@/services/gmail/tokenManager";
