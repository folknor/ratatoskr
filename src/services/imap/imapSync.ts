import { getAccount, updateAccountSyncState } from "../db/accounts";
import { upsertAttachment } from "../db/attachments";
import { withTransaction } from "../db/connection";
import {
  getAllFolderSyncStates,
  upsertFolderSyncState,
} from "../db/folderSyncState";
import { updateMessageThreadIds, upsertMessage } from "../db/messages";
import { getPendingOpsForResource } from "../db/pendingOperations";
import { deleteThread, setThreadLabels, upsertThread } from "../db/threads";
import type { SyncResult } from "../email/types";
import type { ParsedMessage } from "../gmail/messageParser";
import {
  buildThreads,
  type ThreadableMessage,
} from "../threading/threadBuilder";
import {
  getSyncableFolders,
  mapFolderToLabel,
  syncFoldersToLabels,
} from "./folderMapper";
import { buildImapConfig } from "./imapConfigBuilder";
import type {
  DeltaCheckRequest,
  DeltaCheckResult,
  ImapFetchResult,
  ImapMessage,
} from "./tauriCommands";
import {
  imapDeltaCheck,
  imapFetchMessages,
  imapFetchNewUids,
  imapGetFolderStatus,
  imapListFolders,
  imapSearchFolder,
} from "./tauriCommands";

// Re-export public API from phase modules so existing imports continue to work
export {
  formatImapDate,
  computeSinceDate,
  imapMessageToParsedMessage,
} from "./imapSyncConvert";
export type {
  ImapSyncProgress,
  ImapSyncProgressCallback,
} from "./imapSyncConvert";
export { isConnectionError } from "./imapSyncFetch";

import { imapMessageToParsedMessage } from "./imapSyncConvert";
import type { ImapSyncProgressCallback } from "./imapSyncConvert";
import { computeSinceDate } from "./imapSyncConvert";
import {
  CHUNK_SIZE,
  CIRCUIT_BREAKER_DELAY_MS,
  CIRCUIT_BREAKER_MAX_FAILURES,
  CIRCUIT_BREAKER_THRESHOLD,
  INTER_FOLDER_DELAY_MS,
  THREAD_BATCH_SIZE,
  delay,
  fetchMessagesInBatches,
  isConnectionError,
} from "./imapSyncFetch";
import { storeThreadsAndMessages } from "./imapSyncStore";

// ---------------------------------------------------------------------------
// Initial sync
// ---------------------------------------------------------------------------

/**
 * Perform initial sync for an IMAP account.
 * Fetches messages from all folders for the past N days.
 */
export async function imapInitialSync(
  accountId: string,
  daysBack: number = 365,
  onProgress?: ImapSyncProgressCallback,
): Promise<SyncResult> {
  const account = await getAccount(accountId);
  if (!account) {
    throw new Error(`Account ${accountId} not found`);
  }

  const config = buildImapConfig(account);

  // Phase 1: List and sync folders
  onProgress?.({ phase: "folders", current: 0, total: 1 });
  const allFolders = await imapListFolders(config);
  const syncableFolders = getSyncableFolders(allFolders);
  await syncFoldersToLabels(accountId, syncableFolders);
  console.log(
    `[imapSync] Initial sync for account ${accountId}: ${syncableFolders.length} syncable folders`,
  );
  onProgress?.({ phase: "folders", current: 1, total: 1 });

  // ---------------------------------------------------------------------------
  // Phase 2: Streaming fetch & store
  // ---------------------------------------------------------------------------
  // For each folder, for each batch: fetch → parse → store to DB immediately
  // (with placeholder threadId = messageId). Only lightweight metadata is kept
  // in memory for the subsequent threading pass.
  // This avoids accumulating all message bodies in memory (OOM on large mailboxes).

  interface MessageMeta {
    id: string;
    rfcMessageId: string;
    labelIds: string[];
    isRead: boolean;
    isStarred: boolean;
    hasAttachments: boolean;
    subject: string | null;
    snippet: string;
    date: number;
  }

  const allThreadable: ThreadableMessage[] = [];
  const allMeta = new Map<string, MessageMeta>();

  // Track RFC Message-ID → all label IDs from every folder copy.
  // This ensures labels aren't lost when the threading algorithm deduplicates
  // messages that exist in multiple IMAP folders (e.g., INBOX + Sent).
  const labelsByRfcId = new Map<string, Set<string>>();

  // Estimate total messages for progress
  let totalEstimate = 0;
  for (const folder of syncableFolders) {
    totalEstimate += folder.exists;
  }

  let fetchedTotal = 0;
  let totalMessagesFound = 0;
  let storedCount = 0;
  let consecutiveFailures = 0;
  const folderErrors: string[] = [];

  for (let folderIdx = 0; folderIdx < syncableFolders.length; folderIdx++) {
    const folder = syncableFolders[folderIdx];
    if (!folder || folder.exists === 0) continue;

    // Circuit breaker: skip remaining folders after too many consecutive failures
    if (consecutiveFailures >= CIRCUIT_BREAKER_MAX_FAILURES) {
      console.warn(
        `[imapSync] Circuit breaker: ${consecutiveFailures} consecutive connection failures, ` +
          `skipping remaining ${syncableFolders.length - folderIdx} folders`,
      );
      break;
    }

    // Circuit breaker: add cooldown delay after threshold failures
    if (consecutiveFailures >= CIRCUIT_BREAKER_THRESHOLD) {
      console.warn(
        `[imapSync] Circuit breaker: ${consecutiveFailures} consecutive failures, ` +
          `waiting ${CIRCUIT_BREAKER_DELAY_MS / 1000}s before next folder`,
      );
      await delay(CIRCUIT_BREAKER_DELAY_MS);
    }

    // Inter-folder delay to avoid connection bursts (skip before first folder)
    if (folderIdx > 0) {
      await delay(INTER_FOLDER_DELAY_MS);
    }

    const folderMapping = mapFolderToLabel(folder);

    try {
      // Phase 2a: Lightweight search — get UIDs only (no message bodies over IPC)
      const sinceDate = computeSinceDate(daysBack);
      const searchResult = await imapSearchFolder(
        config,
        folder.raw_path,
        sinceDate,
      );
      const uidsToFetch = searchResult.uids;

      // Reset circuit breaker on success
      consecutiveFailures = 0;

      if (uidsToFetch.length === 0) continue;

      // Date filter config
      const cutoffDate = Math.floor(Date.now() / 1000) - daysBack * 86400;
      const nowSeconds = Math.floor(Date.now() / 1000);
      let dateFallbackCount = 0;
      let folderFetchedCount = 0;
      let folderStoredCount = 0;
      let lastUid = 0;
      const uidvalidity = searchResult.folder_status.uidvalidity;

      // Phase 2b: Fetch messages in small IPC-friendly chunks
      for (
        let chunkStart = 0;
        chunkStart < uidsToFetch.length;
        chunkStart += CHUNK_SIZE
      ) {
        const chunkUids = uidsToFetch.slice(
          chunkStart,
          chunkStart + CHUNK_SIZE,
        );
        let chunkResult: ImapFetchResult | undefined;
        try {
          chunkResult = await imapFetchMessages(
            config,
            folder.raw_path,
            chunkUids,
          );
        } catch (chunkErr) {
          // Retry once for transient connection errors
          if (isConnectionError(chunkErr)) {
            console.warn(
              `[imapSync] Chunk fetch failed in ${folder.path}, retrying in 2s:`,
              chunkErr,
            );
            await delay(2_000);
            try {
              chunkResult = await imapFetchMessages(
                config,
                folder.raw_path,
                chunkUids,
              );
            } catch (retryErr) {
              console.error(
                `[imapSync] Chunk retry failed in ${folder.path}:`,
                retryErr,
              );
              continue;
            }
          } else {
            console.error(
              `[imapSync] Failed to fetch chunk ${chunkStart}-${chunkStart + chunkUids.length} in ${folder.path}:`,
              chunkErr,
            );
            continue;
          }
        }

        // Collect parsed data for this chunk to write in a single transaction
        const chunkParsed: {
          parsed: ParsedMessage;
          msg: ImapMessage;
          threadable: ThreadableMessage;
        }[] = [];

        for (const msg of chunkResult.messages) {
          if (msg.uid > lastUid) lastUid = msg.uid;
          folderFetchedCount++;

          // Date filter
          if (msg.date === 0) {
            dateFallbackCount++;
            msg.date = nowSeconds;
          }
          if (msg.date < cutoffDate) continue;

          const { parsed, threadable } = imapMessageToParsedMessage(
            msg,
            accountId,
            folderMapping.labelId,
          );

          parsed.threadId = parsed.id; // placeholder — updated after threading
          chunkParsed.push({ parsed, msg, threadable });
        }

        // Write entire chunk to DB in a single transaction
        if (chunkParsed.length > 0) {
          await withTransaction(async () => {
            for (const { parsed, msg } of chunkParsed) {
              // Create placeholder thread first to satisfy FK constraint
              await upsertThread({
                id: parsed.id,
                accountId,
                subject: parsed.subject,
                snippet: parsed.snippet,
                lastMessageAt: parsed.date,
                messageCount: 1,
                isRead: parsed.isRead,
                isStarred: parsed.isStarred,
                isImportant: false,
                hasAttachments: parsed.hasAttachments,
              });
              await upsertMessage({
                id: parsed.id,
                accountId,
                threadId: parsed.id,
                fromAddress: parsed.fromAddress,
                fromName: parsed.fromName,
                toAddresses: parsed.toAddresses,
                ccAddresses: parsed.ccAddresses,
                bccAddresses: parsed.bccAddresses,
                replyTo: parsed.replyTo,
                subject: parsed.subject,
                snippet: parsed.snippet,
                date: parsed.date,
                isRead: parsed.isRead,
                isStarred: parsed.isStarred,
                bodyHtml: parsed.bodyHtml,
                bodyText: parsed.bodyText,
                rawSize: parsed.rawSize,
                internalDate: parsed.internalDate,
                listUnsubscribe: parsed.listUnsubscribe,
                listUnsubscribePost: parsed.listUnsubscribePost,
                authResults: parsed.authResults,
                messageIdHeader: msg.message_id ?? null,
                referencesHeader: msg.references ?? null,
                inReplyToHeader: msg.in_reply_to ?? null,
                imapUid: msg.uid ?? null,
                imapFolder: msg.folder ?? null,
              });

              // Store attachments
              for (const att of parsed.attachments) {
                await upsertAttachment({
                  id: `${parsed.id}_${att.gmailAttachmentId}`,
                  messageId: parsed.id,
                  accountId,
                  filename: att.filename,
                  mimeType: att.mimeType,
                  size: att.size,
                  gmailAttachmentId: att.gmailAttachmentId,
                  contentId: att.contentId,
                  isInline: att.isInline,
                });
              }
            }
          });
        }

        // Keep only lightweight data in memory for threading
        for (const { parsed, threadable } of chunkParsed) {
          const meta: MessageMeta = {
            id: parsed.id,
            rfcMessageId: threadable.messageId,
            labelIds: parsed.labelIds,
            isRead: parsed.isRead,
            isStarred: parsed.isStarred,
            hasAttachments: parsed.hasAttachments,
            subject: parsed.subject,
            snippet: parsed.snippet,
            date: parsed.date,
          };
          allMeta.set(parsed.id, meta);
          allThreadable.push(threadable);

          // Build cross-folder label map
          let labels = labelsByRfcId.get(threadable.messageId);
          if (!labels) {
            labels = new Set();
            labelsByRfcId.set(threadable.messageId, labels);
          }
          for (const lid of parsed.labelIds) {
            labels.add(lid);
          }
        }

        folderStoredCount += chunkParsed.length;
        storedCount += chunkParsed.length;

        // Report progress after each chunk (not just each folder)
        onProgress?.({
          phase: "messages",
          current:
            fetchedTotal +
            Math.min(chunkStart + CHUNK_SIZE, uidsToFetch.length),
          total: totalEstimate,
          folder: folder.path,
        });
      }

      totalMessagesFound += folderFetchedCount;
      fetchedTotal += uidsToFetch.length;

      if (dateFallbackCount > 0) {
        console.warn(
          `[imapSync] Folder ${folder.path}: ${dateFallbackCount}/${folderFetchedCount} messages had unparseable dates, using current time as fallback`,
        );
      }

      console.log(
        `[imapSync] Folder ${folder.path}: ${uidsToFetch.length} UIDs, ${folderFetchedCount} fetched, ${folderStoredCount} after date filter`,
      );

      // Update folder sync state
      await upsertFolderSyncState({
        account_id: accountId,
        folder_path: folder.raw_path,
        uidvalidity,
        last_uid: lastUid,
        modseq: null,
        last_sync_at: Math.floor(Date.now() / 1000),
      });
    } catch (err) {
      const errMsg =
        err instanceof Error ? err.message : String(err ?? "Unknown error");
      console.error(`[imapSync] Failed to sync folder ${folder.path}:`, err);
      folderErrors.push(`${folder.path}: ${errMsg}`);
      if (isConnectionError(err)) {
        consecutiveFailures++;
      }
      // Continue with next folder
    }
  }

  // If no messages were stored and every folder failed, propagate the error
  if (storedCount === 0 && folderErrors.length > 0) {
    throw new Error(`All folders failed to sync: ${folderErrors[0]}`);
  }

  // ---------------------------------------------------------------------------
  // Phase 3: Thread messages (lightweight — only IDs + headers in memory)
  // ---------------------------------------------------------------------------
  onProgress?.({ phase: "threading", current: 0, total: allThreadable.length });
  const threadGroups = buildThreads(allThreadable);
  console.log(
    `[imapSync] Threading: ${allThreadable.length} messages → ${threadGroups.length} thread groups`,
  );

  // ---------------------------------------------------------------------------
  // Phase 4: Create thread records + batch-update message thread IDs
  // ---------------------------------------------------------------------------
  onProgress?.({
    phase: "storing_threads",
    current: 0,
    total: threadGroups.length,
  });

  for (
    let batchStart = 0;
    batchStart < threadGroups.length;
    batchStart += THREAD_BATCH_SIZE
  ) {
    const batch = threadGroups.slice(
      batchStart,
      batchStart + THREAD_BATCH_SIZE,
    );

    // Pre-check pending ops OUTSIDE the transaction to avoid nested DB issues
    const skippedThreadIds = new Set<string>();
    for (const group of batch) {
      const pendingOps = await getPendingOpsForResource(
        accountId,
        group.threadId,
      );
      if (pendingOps.length > 0) {
        console.log(
          `[imapSync] Skipping thread ${group.threadId}: has ${pendingOps.length} pending local ops`,
        );
        skippedThreadIds.add(group.threadId);
      }
    }

    await withTransaction(async () => {
      for (const group of batch) {
        if (skippedThreadIds.has(group.threadId)) continue;

        const messages = group.messageIds
          .map((id) => allMeta.get(id))
          .filter((m): m is MessageMeta => m !== undefined);

        if (messages.length === 0) continue;

        // Sort by date ascending
        messages.sort((a, b) => a.date - b.date);

        const firstMessage = messages[0];
        const lastMessage = messages[messages.length - 1];
        if (!(firstMessage && lastMessage)) continue;

        // Collect all label IDs including cross-folder copies
        const allLabelIds = new Set<string>();
        for (const msg of messages) {
          for (const lid of msg.labelIds) {
            allLabelIds.add(lid);
          }
          const extraLabels = labelsByRfcId.get(msg.rfcMessageId);
          if (extraLabels) {
            for (const lid of extraLabels) {
              allLabelIds.add(lid);
            }
          }
        }

        const isRead = messages.every((m) => m.isRead);
        const isStarred = messages.some((m) => m.isStarred);
        const hasAttachments = messages.some((m) => m.hasAttachments);

        await upsertThread({
          id: group.threadId,
          accountId,
          subject: firstMessage.subject,
          snippet: lastMessage.snippet,
          lastMessageAt: lastMessage.date,
          messageCount: messages.length,
          isRead,
          isStarred,
          isImportant: false,
          hasAttachments,
        });

        await setThreadLabels(accountId, group.threadId, [...allLabelIds]);

        // Batch-update thread IDs for all messages in this thread
        const messageIds = messages.map((m) => m.id);
        await updateMessageThreadIds(accountId, messageIds, group.threadId);
      }
    });

    onProgress?.({
      phase: "storing_threads",
      current: Math.min(batchStart + THREAD_BATCH_SIZE, threadGroups.length),
      total: threadGroups.length,
    });
  }

  // ---------------------------------------------------------------------------
  // Phase 5: Clean up orphaned placeholder threads
  // ---------------------------------------------------------------------------
  // Phase 2 created a placeholder thread per message (threadId = messageId).
  // Phase 4 merged messages into real threads and updated message thread IDs.
  // Placeholder threads that are no longer referenced by any final thread group
  // should be deleted to avoid ghost threads in the UI.
  const finalThreadIds = new Set(threadGroups.map((g) => g.threadId));
  const allMessageIds = new Set(allMeta.keys());
  let orphanCount = 0;
  for (const msgId of allMessageIds) {
    // If this message's placeholder ID isn't a final thread ID, it's orphaned
    if (!finalThreadIds.has(msgId)) {
      await deleteThread(accountId, msgId);
      orphanCount++;
    }
  }
  if (orphanCount > 0) {
    console.log(
      `[imapSync] Cleaned up ${orphanCount} orphaned placeholder threads`,
    );
  }

  console.log(
    `[imapSync] Stored ${storedCount} messages in ${threadGroups.length} threads (found ${totalMessagesFound} on server)`,
  );

  // Only mark sync as complete if messages were stored OR no messages exist on server.
  if (storedCount > 0 || totalMessagesFound === 0) {
    await updateAccountSyncState(accountId, `imap-synced-${Date.now()}`);
  } else {
    console.warn(
      `[imapSync] Found ${totalMessagesFound} messages on server but stored 0 — NOT marking sync as complete so it will be retried`,
    );
  }

  onProgress?.({
    phase: "done",
    current: storedCount,
    total: storedCount,
  });

  return { messages: [] };
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

/**
 * Perform delta sync for an IMAP account.
 * Fetches only new messages since the last sync using stored UID state.
 */
export async function imapDeltaSync(
  accountId: string,
  daysBack: number = 365,
): Promise<SyncResult> {
  const account = await getAccount(accountId);
  if (!account) {
    throw new Error(`Account ${accountId} not found`);
  }

  const config = buildImapConfig(account);

  // Get all folders we've synced before
  const syncStates = await getAllFolderSyncStates(accountId);

  // Also check for any new folders
  const allFolders = await imapListFolders(config);
  const syncableFolders = getSyncableFolders(allFolders);
  await syncFoldersToLabels(accountId, syncableFolders);

  const syncStateMap = new Map(syncStates.map((s) => [s.folder_path, s]));

  const allParsed = new Map<string, ParsedMessage>();
  const allThreadable: ThreadableMessage[] = [];
  const allImapMsgs = new Map<string, ImapMessage>();

  // Separate folders into new (no saved state) vs existing (have saved state)
  const newFolders = syncableFolders.filter(
    (f) => !syncStateMap.has(f.raw_path),
  );
  const existingFolders = syncableFolders.filter((f) =>
    syncStateMap.has(f.raw_path),
  );

  // Handle new folders: search for UIDs then fetch in chunks
  let consecutiveFailures = 0;
  const deltaFolderErrors: string[] = [];
  for (const folder of newFolders) {
    // Circuit breaker: skip remaining new folders after too many failures
    if (consecutiveFailures >= CIRCUIT_BREAKER_MAX_FAILURES) {
      console.warn(
        `[imapSync] Delta sync circuit breaker: ${consecutiveFailures} consecutive failures, skipping remaining new folders`,
      );
      break;
    }
    if (consecutiveFailures >= CIRCUIT_BREAKER_THRESHOLD) {
      await delay(CIRCUIT_BREAKER_DELAY_MS);
    }

    const folderMapping = mapFolderToLabel(folder);
    try {
      const sinceDate = computeSinceDate(daysBack);
      const searchResult = await imapSearchFolder(
        config,
        folder.raw_path,
        sinceDate,
      );
      consecutiveFailures = 0;

      if (searchResult.uids.length === 0) continue;

      const { messages, lastUid } = await fetchMessagesInBatches(
        config,
        folder.raw_path,
        searchResult.uids,
      );

      for (const msg of messages) {
        const { parsed, threadable } = imapMessageToParsedMessage(
          msg,
          accountId,
          folderMapping.labelId,
        );
        allParsed.set(parsed.id, parsed);
        allThreadable.push(threadable);
        allImapMsgs.set(parsed.id, msg);
      }

      await upsertFolderSyncState({
        account_id: accountId,
        folder_path: folder.raw_path,
        uidvalidity: searchResult.folder_status.uidvalidity,
        last_uid: lastUid,
        modseq: null,
        last_sync_at: Math.floor(Date.now() / 1000),
      });
    } catch (err) {
      const errMsg =
        err instanceof Error ? err.message : String(err ?? "Unknown error");
      console.error(`Delta sync failed for new folder ${folder.path}:`, err);
      deltaFolderErrors.push(`${folder.path}: ${errMsg}`);
      if (isConnectionError(err)) {
        consecutiveFailures++;
      }
    }
  }

  // Batch-check existing folders in a single IMAP connection.
  // Falls back to per-folder checks if the batch command fails.
  if (existingFolders.length > 0) {
    const deltaRequests: DeltaCheckRequest[] = existingFolders.map((folder) => {
      // biome-ignore lint/style/noNonNullAssertion: existingFolders are only folders that exist in syncStateMap
      const savedState = syncStateMap.get(folder.raw_path)!;
      return {
        folder: folder.raw_path,
        last_uid: savedState.last_uid,
        uidvalidity: savedState.uidvalidity ?? 0,
      };
    });

    let deltaResultMap: Map<string, DeltaCheckResult>;
    try {
      const deltaResults = await imapDeltaCheck(config, deltaRequests);
      deltaResultMap = new Map(deltaResults.map((r) => [r.folder, r]));
      console.log(
        `[imapSync] Batch delta check: ${deltaResults.length}/${existingFolders.length} folders checked`,
      );
    } catch (err) {
      // Batch check failed — fall back to per-folder checks
      console.warn(
        `[imapSync] Batch delta check failed, falling back to per-folder:`,
        err,
      );
      deltaResultMap = new Map();
      for (const folder of existingFolders) {
        // biome-ignore lint/style/noNonNullAssertion: existingFolders are only folders that exist in syncStateMap
        const savedState = syncStateMap.get(folder.raw_path)!;
        try {
          const currentStatus = await imapGetFolderStatus(
            config,
            folder.raw_path,
          );
          const uidvalidityChanged =
            savedState.uidvalidity !== null &&
            currentStatus.uidvalidity !== savedState.uidvalidity;

          if (uidvalidityChanged) {
            deltaResultMap.set(folder.raw_path, {
              folder: folder.raw_path,
              uidvalidity: currentStatus.uidvalidity,
              new_uids: [],
              uidvalidity_changed: true,
            });
          } else {
            const newUids = await imapFetchNewUids(
              config,
              folder.raw_path,
              savedState.last_uid,
            );
            deltaResultMap.set(folder.raw_path, {
              folder: folder.raw_path,
              uidvalidity: currentStatus.uidvalidity,
              new_uids: newUids,
              uidvalidity_changed: false,
            });
          }
        } catch (folderErr) {
          console.error(
            `[imapSync] Per-folder check failed for ${folder.path}:`,
            folderErr,
          );
        }
      }
    }

    for (const folder of existingFolders) {
      const folderMapping = mapFolderToLabel(folder);
      // biome-ignore lint/style/noNonNullAssertion: existingFolders are only folders that exist in syncStateMap
      const savedState = syncStateMap.get(folder.raw_path)!;
      const deltaResult = deltaResultMap.get(folder.raw_path);

      if (!deltaResult) continue;

      try {
        if (deltaResult.uidvalidity_changed) {
          // UIDVALIDITY changed — full resync of this folder
          console.warn(
            `UIDVALIDITY changed for folder ${folder.path} ` +
              `(was ${savedState.uidvalidity}, now ${deltaResult.uidvalidity}). ` +
              `Doing full resync of this folder.`,
          );
          const sinceDate = computeSinceDate(daysBack);
          const searchResult = await imapSearchFolder(
            config,
            folder.raw_path,
            sinceDate,
          );
          if (searchResult.uids.length === 0) continue;

          const { messages: resyncMessages, lastUid: resyncLastUid } =
            await fetchMessagesInBatches(
              config,
              folder.raw_path,
              searchResult.uids,
            );

          for (const msg of resyncMessages) {
            const { parsed, threadable } = imapMessageToParsedMessage(
              msg,
              accountId,
              folderMapping.labelId,
            );
            allParsed.set(parsed.id, parsed);
            allThreadable.push(threadable);
            allImapMsgs.set(parsed.id, msg);
          }

          await upsertFolderSyncState({
            account_id: accountId,
            folder_path: folder.raw_path,
            uidvalidity: searchResult.folder_status.uidvalidity,
            last_uid: resyncLastUid,
            modseq: null,
            last_sync_at: Math.floor(Date.now() / 1000),
          });
          continue;
        }

        // Normal delta: fetch the new UIDs returned by delta check
        if (deltaResult.new_uids.length === 0) continue;

        const {
          messages: deltaMessages,
          lastUid: deltaLastUid,
          uidvalidity,
        } = await fetchMessagesInBatches(
          config,
          folder.raw_path,
          deltaResult.new_uids,
        );

        for (const msg of deltaMessages) {
          const { parsed, threadable } = imapMessageToParsedMessage(
            msg,
            accountId,
            folderMapping.labelId,
          );
          allParsed.set(parsed.id, parsed);
          allThreadable.push(threadable);
          allImapMsgs.set(parsed.id, msg);
        }

        await upsertFolderSyncState({
          account_id: accountId,
          folder_path: folder.raw_path,
          uidvalidity,
          last_uid: Math.max(savedState.last_uid, deltaLastUid),
          modseq: null,
          last_sync_at: Math.floor(Date.now() / 1000),
        });
      } catch (err) {
        const errMsg =
          err instanceof Error ? err.message : String(err ?? "Unknown error");
        console.error(`Delta sync failed for folder ${folder.path}:`, err);
        deltaFolderErrors.push(`${folder.path}: ${errMsg}`);
      }
    }
  }

  // If no new messages found and every folder errored, propagate the error
  if (allThreadable.length === 0 && deltaFolderErrors.length > 0) {
    throw new Error(`All folders failed to sync: ${deltaFolderErrors[0]}`);
  }

  if (allThreadable.length === 0) {
    return { messages: [] };
  }

  // Build RFC Message-ID → labels map for cross-folder label merging
  const labelsByRfcId = new Map<string, Set<string>>();
  for (const threadable of allThreadable) {
    const parsed = allParsed.get(threadable.id);
    if (!parsed) continue;
    let labels = labelsByRfcId.get(threadable.messageId);
    if (!labels) {
      labels = new Set();
      labelsByRfcId.set(threadable.messageId, labels);
    }
    for (const lid of parsed.labelIds) {
      labels.add(lid);
    }
  }

  // Thread the new messages
  const threadGroups = buildThreads(allThreadable);

  // Store in DB
  const storedMessages = await storeThreadsAndMessages(
    accountId,
    threadGroups,
    allParsed,
    allImapMsgs,
    labelsByRfcId,
  );

  // Update sync state timestamp
  await updateAccountSyncState(accountId, `imap-synced-${Date.now()}`);

  return { messages: storedMessages };
}
