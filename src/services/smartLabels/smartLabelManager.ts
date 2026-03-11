import { invoke } from "@tauri-apps/api/core";
import { getMessagesByIds } from "@/services/db/messages";
import { dbMessageToParsedMessage } from "@/services/filters/filterEngine";
import type { ParsedMessage } from "@/services/gmail/messageParser";
import {
  classifySmartLabelRemainder,
  matchSmartLabels,
  type SmartLabelAIRule,
  type SmartLabelAIThread,
} from "./smartLabelService";

/**
 * Apply smart labels to newly synced messages.
 * Non-blocking — all errors are caught and logged.
 */
export async function applySmartLabelsToMessages(
  accountId: string,
  messages: ParsedMessage[],
  preAppliedMatches: { threadId: string; labelIds: string[] }[] = [],
  aiRemainder?: {
    threads: SmartLabelAIThread[];
    rules: SmartLabelAIRule[];
  },
): Promise<void> {
  try {
    const matches =
      aiRemainder != null
        ? await classifySmartLabelRemainder(
            aiRemainder.threads,
            aiRemainder.rules,
          )
        : await matchSmartLabels(accountId, messages, preAppliedMatches);
    if (matches.length === 0) return;

    await invoke("smart_labels_apply_matches", {
      accountId,
      matches,
    });
  } catch (err) {
    console.error("Smart label application failed:", err);
  }
}

/**
 * Load messages by IDs from DB, apply smart labels. Used by Rust sync post-sync hooks.
 */
export async function applySmartLabelsToNewMessageIds(
  accountId: string,
  messageIds: string[],
  preAppliedMatches: { threadId: string; labelIds: string[] }[] = [],
  aiRemainder?: {
    threads: SmartLabelAIThread[];
    rules: SmartLabelAIRule[];
  },
): Promise<void> {
  if (aiRemainder != null) {
    await applySmartLabelsToMessages(
      accountId,
      [],
      preAppliedMatches,
      aiRemainder,
    );
    return;
  }

  if (messageIds.length === 0) return;
  const rows = await getMessagesByIds(accountId, messageIds);
  if (rows.length === 0) return;
  const messages = rows.map(dbMessageToParsedMessage);
  await applySmartLabelsToMessages(accountId, messages, preAppliedMatches);
}
