import {
  createDraft as createDraftAction,
  updateDraft as updateDraftAction,
} from "@/services/emailActions";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { buildRawEmail } from "@/utils/emailBuilder";

let debounceTimer: ReturnType<typeof setTimeout> | null = null;
let unsubscribe: (() => void) | null = null;

const DEBOUNCE_MS = 3000;

async function saveDraft(): Promise<void> {
  const state = useComposerStore.getState();
  // Read the active account ID at save time (not capture time) so that
  // switching accounts during the 3s debounce window doesn't save to the wrong account.
  const accountId = useAccountStore.getState().activeAccountId;
  if (!(state.isOpen && accountId)) return;

  const accounts = useAccountStore.getState().accounts;
  const account = accounts.find((a) => a.id === accountId);
  if (!account) return;

  // Don't save empty drafts
  if (!(state.bodyHtml || state.subject) && state.to.length === 0) return;

  state.setIsSaving(true);

  try {
    const raw = buildRawEmail({
      from: account.email,
      to: state.to.length > 0 ? state.to : [""],
      subject: state.subject,
      htmlBody: state.bodyHtml,
      threadId: state.threadId ?? undefined,
      attachments:
        state.attachments.length > 0
          ? state.attachments.map((a) => ({
              filename: a.filename,
              mimeType: a.mimeType,
              content: a.content,
            }))
          : undefined,
    });

    if (state.draftId) {
      await updateDraftAction(
        accountId,
        state.draftId,
        raw,
        state.threadId ?? undefined,
      );
    } else {
      const result = await createDraftAction(
        accountId,
        raw,
        state.threadId ?? undefined,
      );
      if (
        result.data &&
        typeof result.data === "object" &&
        "draftId" in result.data
      ) {
        state.setDraftId((result.data as { draftId: string }).draftId);
      }
    }

    state.setLastSavedAt(Date.now());
  } catch (err) {
    console.error("Failed to auto-save draft:", err);
  } finally {
    state.setIsSaving(false);
  }
}

function scheduleSave(): void {
  if (debounceTimer) clearTimeout(debounceTimer);
  debounceTimer = setTimeout(saveDraft, DEBOUNCE_MS);
}

/**
 * Start watching composerStore changes and auto-saving drafts.
 */
export function startAutoSave(_accountId?: string): void {
  // Note: _accountId is accepted for backward compatibility but ignored.
  // The active account ID is read from the account store at save time to
  // prevent saving to the wrong account if the user switches during debounce.
  stopAutoSave();

  // Subscribe to store changes — trigger debounced save on any field change
  unsubscribe = useComposerStore.subscribe((state, prevState) => {
    if (!state.isOpen) return;
    // Only save when content-relevant fields change
    if (
      state.bodyHtml !== prevState.bodyHtml ||
      state.subject !== prevState.subject ||
      state.to !== prevState.to ||
      state.cc !== prevState.cc ||
      state.bcc !== prevState.bcc ||
      state.attachments !== prevState.attachments
    ) {
      scheduleSave();
    }
  });
}

/**
 * Stop auto-saving and clean up.
 */
export function stopAutoSave(): void {
  if (debounceTimer) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
  if (unsubscribe) {
    unsubscribe();
    unsubscribe = null;
  }
}
