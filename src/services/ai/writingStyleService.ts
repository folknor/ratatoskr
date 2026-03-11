import { getAccountBasicInfo } from "@/services/accounts/basicInfo";
import { deleteAiCache, getAiCache, setAiCache } from "@/services/db/aiCache";
import { type DbMessage, getRecentSentMessages } from "@/services/db/messages";
import { getSetting } from "@/services/db/settings";
import {
  deleteWritingStyleProfile,
  getWritingStyleProfile,
  upsertWritingStyleProfile,
} from "@/services/db/writingStyleProfiles";
import { completeAi, testAiConnection } from "./client";
import {
  AUTO_DRAFT_REPLY_PROMPT,
  WRITING_STYLE_ANALYSIS_PROMPT,
} from "./prompts";

async function callAi(
  systemPrompt: string,
  userContent: string,
): Promise<string> {
  return completeAi({ systemPrompt, userContent });
}

/**
 * Analyze writing style from sent email samples.
 */
export async function analyzeWritingStyle(
  samples: DbMessage[],
): Promise<string> {
  const formatted = samples
    .map((msg) => {
      const body = (msg.body_text ?? msg.snippet ?? "").trim().slice(0, 1000);
      return `--- Sample ---\n${body}`;
    })
    .join("\n\n");

  return callAi(WRITING_STYLE_ANALYSIS_PROMPT, formatted.slice(0, 8000));
}

/**
 * Get existing style profile or create one by analyzing recent sent emails.
 */
export async function getOrCreateStyleProfile(
  accountId: string,
): Promise<string | null> {
  const styleEnabled = await getSetting("ai_writing_style_enabled");
  if (styleEnabled === "false") return null;

  // Check for cached profile
  const existing = await getWritingStyleProfile(accountId);
  if (existing) return existing.profile_text;

  // Get account email for matching sent messages
  const account = await getAccountBasicInfo(accountId);
  if (!account) return null;

  // Fetch recent sent messages
  const sentMessages = await getRecentSentMessages(
    accountId,
    account.email,
    15,
  );
  if (sentMessages.length < 3) return null; // Not enough samples

  // Analyze and cache
  const profileText = await analyzeWritingStyle(sentMessages);
  await upsertWritingStyleProfile(accountId, profileText, sentMessages.length);
  return profileText;
}

/**
 * Force re-analysis of writing style from latest sent emails.
 */
export async function refreshWritingStyle(
  accountId: string,
): Promise<string | null> {
  await deleteWritingStyleProfile(accountId);
  return getOrCreateStyleProfile(accountId);
}

function formatThreadForDraft(messages: DbMessage[]): string {
  return messages
    .map((msg) => {
      const from = msg.from_name
        ? `${msg.from_name} <${msg.from_address}>`
        : (msg.from_address ?? "Unknown");
      const date = new Date(msg.date).toLocaleDateString("en-US", {
        month: "short",
        day: "numeric",
        year: "numeric",
      });
      const body = (msg.body_text ?? msg.snippet ?? "").trim();
      return `From: ${from}\nDate: ${date}\n\n${body}`;
    })
    .join("\n---\n");
}

export type AutoDraftMode = "reply" | "replyAll";

/**
 * Generate an auto-draft reply for a thread.
 * Returns cached version if available.
 */
export async function generateAutoDraft(
  threadId: string,
  accountId: string,
  messages: DbMessage[],
  mode: AutoDraftMode,
): Promise<string> {
  const cacheType = `auto_draft_${mode}`;

  // Check cache
  const cached = await getAiCache(accountId, threadId, cacheType);
  if (cached) return cached;

  // Get writing style profile (lazy creation)
  const styleProfile = await getOrCreateStyleProfile(accountId);

  // Build the prompt
  const subject = messages[0]?.subject ?? "No subject";
  const threadContent = formatThreadForDraft(messages);
  const styleSection = styleProfile
    ? `\n\nUser's writing style:\n${styleProfile}`
    : "";

  const userContent =
    `<email_content>Subject: ${subject}\n\n${threadContent}</email_content>${styleSection}`.slice(
      0,
      6000,
    );

  const draft = await callAi(AUTO_DRAFT_REPLY_PROMPT, userContent);

  // Cache the result
  await setAiCache(accountId, threadId, cacheType, draft);
  return draft;
}

/**
 * Regenerate auto-draft (clear cache and generate fresh).
 */
export async function regenerateAutoDraft(
  threadId: string,
  accountId: string,
  messages: DbMessage[],
  mode: AutoDraftMode,
): Promise<string> {
  const cacheType = `auto_draft_${mode}`;
  await deleteAiCache(accountId, threadId, cacheType);
  return generateAutoDraft(threadId, accountId, messages, mode);
}

/**
 * Check if auto-draft is available (AI configured + setting enabled).
 */
export async function isAutoDraftEnabled(): Promise<boolean> {
  const enabled = await getSetting("ai_auto_draft_enabled");
  if (enabled === "false") return false;

  try {
    return await testAiConnection();
  } catch {
    return false;
  }
}
