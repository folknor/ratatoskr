/**
 * Core AI facade — re-exports every AI-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ai/* directly.
 */

// AI service functions
export {
  composeFromPrompt,
  generateReply,
  generateSmartReplies,
  summarizeThread,
  testConnection,
  type TransformType,
  transformText,
} from "@/services/ai/aiService";

// Provider management
export {
  clearProviderClients,
  isAiAvailable,
} from "@/services/ai/providerManager";

// AI types
export { PROVIDER_MODELS } from "@/services/ai/types";

// Ask inbox
export { type AskInboxResult, askMyInbox } from "@/services/ai/askInbox";

// Task extraction
export { extractTask } from "@/services/ai/taskExtraction";

// Writing style
export {
  type AutoDraftMode,
  generateAutoDraft,
  isAutoDraftEnabled,
  refreshWritingStyle,
  regenerateAutoDraft,
} from "@/services/ai/writingStyleService";

// AI cache
export { deleteAiCache } from "@/services/db/aiCache";
