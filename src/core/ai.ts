/**
 * Core AI facade — re-exports every AI-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ai/* directly.
 */

// AI service functions
// biome-ignore lint/performance/noBarrelFile: Intentional UI-facing facade for AI APIs.
export {
  composeFromPrompt,
  generateReply,
  generateSmartReplies,
  summarizeThread,
  type TransformType,
  testConnection,
  transformText,
} from "@/services/ai/aiService";
// Ask inbox
export { type AskInboxResult, askMyInbox } from "@/services/ai/askInbox";
// Provider management
export { isAiAvailable } from "@/services/ai/providerManager";
// Task extraction
export { extractTask } from "@/services/ai/taskExtraction";
// AI types
export { PROVIDER_MODELS } from "@/services/ai/types";

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
