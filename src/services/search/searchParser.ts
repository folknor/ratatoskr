/**
 * Parses search query strings with operator support.
 * Supported operators: from:, to:, subject:, has:attachment, is:unread, is:read,
 * is:starred, before:, after:, label:
 */

import { parseDateToTimestamp } from "@/utils/date";

export interface ParsedSearchQuery {
  freeText: string;
  from?: string;
  to?: string;
  subject?: string;
  hasAttachment?: boolean;
  isUnread?: boolean;
  isRead?: boolean;
  isStarred?: boolean;
  before?: number; // unix timestamp (seconds)
  after?: number; // unix timestamp (seconds)
  label?: string;
}

const OPERATOR_REGEX =
  /(?:^|\s)(from|to|subject|has|is|before|after|label):\s*(?:"([^"]+)"|(\S+))/gi;

export function parseSearchQuery(input: string): ParsedSearchQuery {
  const result: ParsedSearchQuery = { freeText: "" };

  // Extract operators and collect remaining free text
  let remaining = input;
  let match: RegExpExecArray | null;

  // Reset regex lastIndex
  OPERATOR_REGEX.lastIndex = 0;

  const matches: { start: number; end: number }[] = [];

  for (
    match = OPERATOR_REGEX.exec(input);
    match !== null;
    match = OPERATOR_REGEX.exec(input)
  ) {
    const operator = match[1]?.toLowerCase();
    const value = match[2] ?? match[3] ?? "";

    matches.push({ start: match.index, end: match.index + match[0].length });

    switch (operator) {
      case "from":
        result.from = value;
        break;
      case "to":
        result.to = value;
        break;
      case "subject":
        result.subject = value;
        break;
      case "has":
        if (value.toLowerCase() === "attachment") {
          result.hasAttachment = true;
        }
        break;
      case "is":
        switch (value.toLowerCase()) {
          case "unread":
            result.isUnread = true;
            break;
          case "read":
            result.isRead = true;
            break;
          case "starred":
            result.isStarred = true;
            break;
        }
        break;
      case "before": {
        const ts = parseDateToTimestamp(value);
        if (ts !== undefined) result.before = ts;
        break;
      }
      case "after": {
        const ts = parseDateToTimestamp(value);
        if (ts !== undefined) result.after = ts;
        break;
      }
      case "label":
        result.label = value;
        break;
    }
  }

  // Build free text by removing matched operator segments
  // Process matches in reverse to preserve indices
  remaining = input;
  for (let i = matches.length - 1; i >= 0; i--) {
    const m = matches[i];
    if (!m) continue;
    remaining = remaining.slice(0, m.start) + remaining.slice(m.end);
  }

  result.freeText = remaining.replace(/\s+/g, " ").trim();
  return result;
}

/**
 * Returns true if the query string contains any search operators.
 */
export function hasSearchOperators(query: string): boolean {
  OPERATOR_REGEX.lastIndex = 0;
  return OPERATOR_REGEX.test(query);
}
