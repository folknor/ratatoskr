import type { ImapConfig, ImapMessage } from "./tauriCommands";
import { imapFetchMessages } from "./tauriCommands";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const BATCH_SIZE = 50;
/** Number of messages to fetch per IPC call during initial sync. */
export const CHUNK_SIZE = 200;
/** Number of thread groups to process per transaction in Phase 4. */
export const THREAD_BATCH_SIZE = 100;

// ---------------------------------------------------------------------------
// Circuit breaker for connection storms
// ---------------------------------------------------------------------------

/** After this many consecutive connection failures, add a cooldown delay. */
export const CIRCUIT_BREAKER_THRESHOLD = 3;
/** Delay (ms) to wait after hitting the circuit breaker threshold. */
export const CIRCUIT_BREAKER_DELAY_MS = 15_000;
/** After this many consecutive failures, skip remaining folders entirely. */
export const CIRCUIT_BREAKER_MAX_FAILURES = 5;
/** Delay (ms) between folder syncs during initial sync to avoid connection bursts. */
export const INTER_FOLDER_DELAY_MS = 1_000;

export function isConnectionError(err: unknown): boolean {
  const msg = String(err).toLowerCase();
  return (
    msg.includes("timed out") ||
    msg.includes("connection") ||
    msg.includes("tcp") ||
    msg.includes("tls") ||
    msg.includes("dns") ||
    msg.includes("econnrefused") ||
    msg.includes("network") ||
    msg.includes("socket")
  );
}

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ---------------------------------------------------------------------------
// Fetch messages from a folder in batches
// ---------------------------------------------------------------------------

/**
 * Fetch messages from a folder in batches of BATCH_SIZE.
 */
export async function fetchMessagesInBatches(
  config: ImapConfig,
  folder: string,
  uids: number[],
  onBatch?: (fetched: number, total: number) => void,
): Promise<{ messages: ImapMessage[]; lastUid: number; uidvalidity: number }> {
  const allMessages: ImapMessage[] = [];
  let lastUid = 0;
  let uidvalidity = 0;

  for (let i = 0; i < uids.length; i += BATCH_SIZE) {
    const batch = uids.slice(i, i + BATCH_SIZE);
    const result = await imapFetchMessages(config, folder, batch);

    allMessages.push(...result.messages);
    uidvalidity = result.folder_status.uidvalidity;

    for (const msg of result.messages) {
      if (msg.uid > lastUid) lastUid = msg.uid;
    }

    onBatch?.(Math.min(i + BATCH_SIZE, uids.length), uids.length);
  }

  return { messages: allMessages, lastUid, uidvalidity };
}
