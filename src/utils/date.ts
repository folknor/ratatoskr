/**
 * Format a unix timestamp (milliseconds) into a relative date string.
 */
export function formatRelativeDate(timestamp: number): string {
  const date = new Date(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffDays = Math.floor(diffMs / 86_400_000);

  // Today: show time
  if (isSameDay(date, now)) {
    return date.toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
    });
  }

  // Yesterday
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (isSameDay(date, yesterday)) {
    return "Yesterday";
  }

  // Within last 7 days: show day name
  if (diffDays < 7) {
    return date.toLocaleDateString(undefined, { weekday: "short" });
  }

  // Same year: show month + day
  if (date.getFullYear() === now.getFullYear()) {
    return date.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
  }

  // Older: show full date
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/**
 * Format a unix timestamp into a full date string for message headers.
 */
export function formatFullDate(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

// ---------------------------------------------------------------------------
// Date parsing (used by search operators)
// ---------------------------------------------------------------------------

/**
 * Parse a date string like YYYY/MM/DD or YYYY-MM-DD into a unix timestamp (seconds).
 * Returns undefined if the string is not a valid date.
 */
export function parseDateToTimestamp(dateStr: string): number | undefined {
  const normalized = dateStr.replace(/-/g, "/");
  const parts = normalized.split("/");
  if (parts.length !== 3) return;
  const year = parseInt(parts[0] ?? "", 10);
  const month = parseInt(parts[1] ?? "", 10);
  const day = parseInt(parts[2] ?? "", 10);
  if (Number.isNaN(year) || Number.isNaN(month) || Number.isNaN(day)) return;
  const date = new Date(year, month - 1, day);
  if (Number.isNaN(date.getTime())) return;
  return Math.floor(date.getTime() / 1000);
}

// ---------------------------------------------------------------------------
// IMAP SINCE date helpers
// ---------------------------------------------------------------------------

const IMAP_MONTH_NAMES = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

/**
 * Format a Date as `DD-Mon-YYYY` for the IMAP SINCE search criterion (RFC 3501 §6.4.4).
 */
export function formatImapDate(date: Date): string {
  const day = date.getUTCDate();
  const month = IMAP_MONTH_NAMES[date.getUTCMonth()];
  const year = date.getUTCFullYear();
  return `${day}-${month}-${year}`;
}

/**
 * Compute a `DD-Mon-YYYY` SINCE date string for the given `daysBack` value.
 * Subtracts an extra day as a safety margin for timezone differences
 * (IMAP SINCE has date-only granularity, no time component).
 */
export function computeSinceDate(daysBack: number): string {
  const date = new Date();
  date.setUTCDate(date.getUTCDate() - daysBack - 1);
  return formatImapDate(date);
}
