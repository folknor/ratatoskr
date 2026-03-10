import { bodyStoreGetBatch } from "@/core/rustDb";
import type { FilterActions, FilterCriteria } from "../db/filters";
import { getEnabledFiltersForAccount } from "../db/filters";
import { type DbMessage, getMessagesByIds } from "../db/messages";
import {
  addThreadLabel,
  markThreadRead,
  removeThreadLabel,
  starThread,
} from "../emailActions";
import type { ParsedMessage } from "../gmail/messageParser";

/**
 * Check if a parsed message matches the given filter criteria.
 * All set criteria must match (AND logic). Matching is case-insensitive substring.
 */
export function messageMatchesFilter(
  message: ParsedMessage,
  criteria: FilterCriteria,
): boolean {
  if (criteria.from) {
    const fromStr =
      `${message.fromName ?? ""} ${message.fromAddress ?? ""}`.toLowerCase();
    if (!fromStr.includes(criteria.from.toLowerCase())) return false;
  }

  if (criteria.to) {
    const toStr = (message.toAddresses ?? "").toLowerCase();
    if (!toStr.includes(criteria.to.toLowerCase())) return false;
  }

  if (criteria.subject) {
    const subjectStr = (message.subject ?? "").toLowerCase();
    if (!subjectStr.includes(criteria.subject.toLowerCase())) return false;
  }

  if (criteria.body) {
    const bodyStr =
      `${message.bodyText ?? ""} ${message.bodyHtml ?? ""}`.toLowerCase();
    if (!bodyStr.includes(criteria.body.toLowerCase())) return false;
  }

  if (criteria.hasAttachment) {
    if (!message.hasAttachments) return false;
  }

  return true;
}

export interface FilterResult {
  addLabelIds: string[];
  removeLabelIds: string[];
  markRead: boolean;
  star: boolean;
}

/**
 * Compute the aggregate label/flag changes from a set of filter actions.
 */
export function computeFilterActions(actions: FilterActions): FilterResult {
  const addLabelIds: string[] = [];
  const removeLabelIds: string[] = [];

  if (actions.applyLabel) {
    addLabelIds.push(actions.applyLabel);
  }

  if (actions.archive) {
    removeLabelIds.push("INBOX");
  }

  if (actions.trash) {
    addLabelIds.push("TRASH");
    removeLabelIds.push("INBOX");
  }

  if (actions.star) {
    addLabelIds.push("STARRED");
  }

  return {
    addLabelIds,
    removeLabelIds,
    markRead: actions.markRead ?? false,
    star: actions.star ?? false,
  };
}

/**
 * Apply all enabled filters to a set of new messages for the given account.
 * Modifies threads via the Gmail API and updates local DB.
 */
export async function applyFiltersToMessages(
  accountId: string,
  messages: ParsedMessage[],
): Promise<void> {
  const filters = await getEnabledFiltersForAccount(accountId);
  if (filters.length === 0) return;

  // Pre-parse filter JSON once (not per-message) to avoid O(M×F) parse operations
  const parsedFilters = filters.flatMap((filter) => {
    try {
      return [
        {
          criteria: JSON.parse(filter.criteria_json) as FilterCriteria,
          actions: JSON.parse(filter.actions_json) as FilterActions,
        },
      ];
    } catch {
      return [];
    }
  });
  if (parsedFilters.length === 0) return;

  // Group actions by threadId so we can batch modifications
  const threadActions = new Map<string, FilterResult>();

  for (const msg of messages) {
    for (const { criteria, actions } of parsedFilters) {
      if (messageMatchesFilter(msg, criteria)) {
        const result = computeFilterActions(actions);
        const existing = threadActions.get(msg.threadId);
        if (existing) {
          // Merge results
          existing.addLabelIds.push(...result.addLabelIds);
          existing.removeLabelIds.push(...result.removeLabelIds);
          existing.markRead = existing.markRead || result.markRead;
          existing.star = existing.star || result.star;
        } else {
          threadActions.set(msg.threadId, result);
        }
      }
    }
  }

  // Apply combined actions per thread in parallel
  await Promise.allSettled(
    [...threadActions].map(async ([threadId, result]) => {
      const addLabels = [...new Set(result.addLabelIds)];
      const removeLabels = [...new Set(result.removeLabelIds)];

      try {
        // Apply label changes via provider
        for (const labelId of addLabels) {
          await addThreadLabel(accountId, threadId, labelId);
        }
        for (const labelId of removeLabels) {
          await removeThreadLabel(accountId, threadId, labelId);
        }

        // Mark as read via provider
        if (result.markRead) {
          await markThreadRead(accountId, threadId, [], true);
        }

        // Star via provider
        if (result.star) {
          await starThread(accountId, threadId, [], true);
        }
      } catch (err) {
        console.error(
          `Failed to apply filter actions to thread ${threadId}:`,
          err,
        );
      }
    }),
  );
}

/** Convert a DB message row to a lightweight ParsedMessage for filter matching. */
export function dbMessageToParsedMessage(row: DbMessage): ParsedMessage {
  return {
    id: row.id,
    threadId: row.thread_id,
    fromAddress: row.from_address,
    fromName: row.from_name,
    toAddresses: row.to_addresses,
    ccAddresses: row.cc_addresses,
    bccAddresses: row.bcc_addresses,
    replyTo: row.reply_to,
    subject: row.subject,
    snippet: row.snippet ?? "",
    date: row.date,
    isRead: row.is_read === 1,
    isStarred: row.is_starred === 1,
    bodyHtml: row.body_html,
    bodyText: row.body_text,
    rawSize: row.raw_size ?? 0,
    internalDate: row.internal_date ?? row.date,
    labelIds: [],
    hasAttachments: false, // messages table has no has_attachments column; thread-level only
    attachments: [],
    listUnsubscribe: row.list_unsubscribe,
    listUnsubscribePost: row.list_unsubscribe_post,
    authResults: row.auth_results,
  };
}

/**
 * Load messages by IDs from DB, apply filters. Used by Rust sync post-sync hooks.
 *
 * Message bodies live in a separate body store (bodies.db), not in the messages
 * table. When any active filter uses `criteria.body`, we hydrate bodies from
 * the body store before evaluating filters.
 */
export async function applyFiltersToNewMessageIds(
  accountId: string,
  messageIds: string[],
): Promise<void> {
  if (messageIds.length === 0) return;
  const rows = await getMessagesByIds(accountId, messageIds);
  if (rows.length === 0) return;

  // Check if any filter needs body matching so we can hydrate from body store
  const filters = await getEnabledFiltersForAccount(accountId);
  const needsBody = filters.some((f) => {
    try {
      const criteria = JSON.parse(f.criteria_json) as FilterCriteria;
      return Boolean(criteria.body);
    } catch {
      return false;
    }
  });

  if (needsBody) {
    const bodies = await bodyStoreGetBatch(rows.map((r) => r.id));
    const bodyMap = new Map(bodies.map((b) => [b.messageId, b]));
    for (const row of rows) {
      const body = bodyMap.get(row.id);
      if (body) {
        row.body_html = body.bodyHtml;
        row.body_text = body.bodyText;
      }
    }
  }

  const messages = rows.map(dbMessageToParsedMessage);
  await applyFiltersToMessages(accountId, messages);
}
