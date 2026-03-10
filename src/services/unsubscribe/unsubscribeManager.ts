import { invoke } from "@tauri-apps/api/core";
import { fetch } from "@tauri-apps/plugin-http";
import { openUrl } from "@tauri-apps/plugin-opener";
import { normalizeEmail } from "@/utils/emailUtils";
import { getCurrentUnixTimestamp } from "@/utils/timestamp";

export interface ParsedUnsubscribe {
  httpUrl: string | null;
  mailtoAddress: string | null;
  hasOneClick: boolean;
}

export interface SubscriptionEntry {
  from_address: string;
  from_name: string | null;
  latest_unsubscribe_header: string;
  latest_unsubscribe_post: string | null;
  message_count: number;
  latest_date: number;
  status: string | null;
}

/**
 * Parse List-Unsubscribe and List-Unsubscribe-Post headers into actionable data.
 */
export function parseUnsubscribeHeaders(
  listUnsubscribe: string,
  listUnsubscribePost: string | null,
): ParsedUnsubscribe {
  const httpMatch = listUnsubscribe.match(/<(https?:\/\/[^>]+)>/);
  const mailtoMatch = listUnsubscribe.match(/<mailto:([^>]+)>/);
  const hasOneClick = Boolean(
    listUnsubscribePost?.toLowerCase().includes("list-unsubscribe=one-click"),
  );

  return {
    httpUrl: httpMatch?.[1] ?? null,
    mailtoAddress: mailtoMatch?.[1] ?? null,
    hasOneClick,
  };
}

/**
 * Execute unsubscribe using the best available method:
 * 1. RFC 8058 one-click POST (no browser needed)
 * 2. mailto via Gmail API
 * 3. Fallback: open URL in browser
 */
// biome-ignore lint/complexity/useMaxParams: unsubscribe requires all context fields
export async function executeUnsubscribe(
  accountId: string,
  threadId: string,
  fromAddress: string,
  fromName: string | null,
  listUnsubscribe: string,
  listUnsubscribePost: string | null,
): Promise<{ method: string; success: boolean }> {
  const parsed = parseUnsubscribeHeaders(listUnsubscribe, listUnsubscribePost);

  let method = "browser";
  let success = false;

  // Method 1: RFC 8058 one-click HTTP POST
  if (parsed.hasOneClick && parsed.httpUrl) {
    try {
      const response = await fetch(parsed.httpUrl, {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: new TextEncoder().encode("List-Unsubscribe=One-Click"),
      });
      success =
        response.ok || response.status === 200 || response.status === 202;
      method = "http_post";
    } catch (err) {
      console.error("One-click unsubscribe failed, trying fallback:", err);
    }
  }

  // Method 2: mailto via provider-agnostic send
  if (!success && parsed.mailtoAddress) {
    try {
      const to = parsed.mailtoAddress.split("?")[0] ?? parsed.mailtoAddress;
      // Extract subject from mailto params if present
      const subjectMatch = parsed.mailtoAddress.match(/subject=([^&]+)/i);
      const subject = subjectMatch
        ? decodeURIComponent(subjectMatch[1] ?? "")
        : "unsubscribe";

      const { getAccount } = await import("../db/accounts");
      const account = await getAccount(accountId);
      const { buildRawEmail } = await import("../../utils/emailBuilder");
      const raw = buildRawEmail({
        from: account?.email ?? "",
        to: [to],
        subject,
        htmlBody: "unsubscribe",
      });
      const { sendEmail } = await import("../emailActions");
      await sendEmail(accountId, raw);
      method = "mailto";
      success = true;
    } catch (err) {
      console.error("Mailto unsubscribe failed, trying fallback:", err);
    }
  }

  // Method 3: open in browser
  if (!success && parsed.httpUrl) {
    try {
      await openUrl(parsed.httpUrl);
      method = "browser";
      success = true;
    } catch (err) {
      console.error("Browser unsubscribe failed:", err);
    }
  }

  // Record the action
  await recordUnsubscribeAction(
    accountId,
    threadId,
    fromAddress,
    fromName,
    method,
    parsed.httpUrl ?? parsed.mailtoAddress ?? listUnsubscribe,
    success ? "unsubscribed" : "failed",
  );

  return { method, success };
}

// biome-ignore lint/complexity/useMaxParams: DB record requires all fields as separate params
async function recordUnsubscribeAction(
  accountId: string,
  threadId: string,
  fromAddress: string,
  fromName: string | null,
  method: string,
  url: string,
  status: string,
): Promise<void> {
  const id = crypto.randomUUID();
  const now = getCurrentUnixTimestamp();
  await invoke("db_record_unsubscribe_action", {
    id,
    accountId,
    threadId,
    fromAddress: normalizeEmail(fromAddress),
    fromName,
    method,
    unsubscribeUrl: url,
    status,
    now,
  });
}

/**
 * Get all detectable newsletter/promo subscriptions for an account.
 */
export async function getSubscriptions(
  accountId: string,
): Promise<SubscriptionEntry[]> {
  return invoke("db_get_subscriptions", { accountId });
}

/**
 * Get unsubscribe status for a specific sender.
 */
export async function getUnsubscribeStatus(
  accountId: string,
  fromAddress: string,
): Promise<string | null> {
  return invoke("db_get_unsubscribe_status", {
    accountId,
    fromAddress: normalizeEmail(fromAddress),
  });
}
