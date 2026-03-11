import { getMessagesByIds } from "@/services/db/messages";
import { addThreadLabel } from "@/services/emailActions";
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

    await Promise.allSettled(
      matches.flatMap(({ threadId, labelIds }) =>
        labelIds.map((labelId) =>
          addThreadLabel(accountId, threadId, labelId).catch((err) => {
            console.error(
              `Failed to apply smart label ${labelId} to thread ${threadId}:`,
              err,
            );
          }),
        ),
      ),
    );
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
