import { scanMessage } from "@/utils/phishingDetector";
import type { MessageScanResult, PhishingSensitivity } from "@/utils/phishingDetector";
import { getSetting } from "@/services/db/settings";
import { isPhishingAllowlisted } from "@/services/db/phishingAllowlist";
import { getCachedScanResult, cacheScanResult } from "@/services/db/linkScanResults";

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
  // 1. Check if phishing detection is enabled
  const enabled = await getSetting("phishing_detection_enabled");
  if (enabled === "false") {
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

  // 4. Read sensitivity setting and scan the message
  const sensitivityRaw = await getSetting("phishing_sensitivity");
  const sensitivity: PhishingSensitivity =
    sensitivityRaw === "low" || sensitivityRaw === "high" ? sensitivityRaw : "default";
  const result = scanMessage(messageId, bodyHtml, sensitivity);

  // 5. Cache the result
  try {
    await cacheScanResult(accountId, messageId, JSON.stringify(result));
  } catch (err) {
    console.error("Failed to cache phishing scan result:", err);
  }

  return result;
}
