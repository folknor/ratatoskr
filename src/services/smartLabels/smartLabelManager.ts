import { invoke } from "@tauri-apps/api/core";
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
  provider: string,
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
            preAppliedMatches,
          )
        : await matchSmartLabels(accountId, messages, preAppliedMatches);
    if (matches.length === 0) return;

    await invoke("smart_labels_apply_matches", {
      accountId,
      provider,
      matches,
    });
  } catch (err) {
    console.error("Smart label application failed:", err);
  }
}

/**
 * Apply AI smart-label remainder returned by the Rust sync pipeline.
 */
export async function applySmartLabelsFromAiRemainder(
  accountId: string,
  provider: string,
  preAppliedMatches: { threadId: string; labelIds: string[] }[] = [],
  aiRemainder: {
    threads: SmartLabelAIThread[];
    rules: SmartLabelAIRule[];
  },
): Promise<void> {
  await applySmartLabelsToMessages(
    accountId,
    provider,
    [],
    preAppliedMatches,
    aiRemainder,
  );
}
