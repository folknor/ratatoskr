import {
  cacheScanResult,
  getCachedScanResult,
} from "@/services/db/linkScanResults";
import { isPhishingAllowlisted } from "@/services/db/phishingAllowlist";
import { getPhishingSettings } from "@/services/settings/runtimeFlags";
import type { MessageScanResult } from "@/utils/phishingDetector";
import { scanMessage } from "@/utils/phishingDetector";

/**
 * Orchestrates phishing link scanning for a message.
 *
 * Flow:
 * 1. Check if feature is enabled (setting: phishing_detection_enabled)
 * 2. Check if sender is in the allowlist
 * 3. Check cache for existing result
 * 4. Scan the message HTML
 * 5. Cache the result
 */
export async function scanMessageLinks(
  accountId: string,
  messageId: string,
  bodyHtml: string | null,
  senderAddress: string | null,
): Promise<MessageScanResult | null> {
  const { enabled, sensitivity } = await getPhishingSettings();
  if (!enabled) {
    return null;
  }

  // 2. Check if sender is allowlisted
  if (senderAddress) {
    const allowlisted = await isPhishingAllowlisted(accountId, senderAddress);
    if (allowlisted) {
      return null;
    }
  }

  // 3. Check cache
  const cached = await getCachedScanResult(accountId, messageId);
  if (cached) {
    try {
      return JSON.parse(cached) as MessageScanResult;
    } catch {
      // Invalid cache entry — proceed with fresh scan
    }
  }

  const result = scanMessage(messageId, bodyHtml, sensitivity);

  // 5. Cache the result
  try {
    await cacheScanResult(accountId, messageId, JSON.stringify(result));
  } catch (err) {
    console.error("Failed to cache phishing scan result:", err);
  }

  return result;
}
