import {
  createDraft as createDraftAction,
  updateDraft as updateDraftAction,
} from "@/services/emailActions";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { buildRawEmail } from "@/utils/emailBuilder";

let debounceTimer: ReturnType<typeof setTimeout> | null = null;
let unsubscribe: (() => void) | null = null;
let saveInFlight: Promise<void> | null = null;

const DEBOUNCE_MS = 3000;

async function saveDraft(): Promise<void> {
  const state = useComposerStore.getState();
  // Use the account ID captured when the composer was opened, falling back to
  // the active account. This prevents saving to the wrong account if the user
  // switches accounts during the 3s debounce window.
  const accountId =
    state.accountId ?? useAccountStore.getState().activeAccountId;
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
      const result = await updateDraftAction(
        accountId,
        state.draftId,
        raw,
        state.threadId ?? undefined,
      );
      // Update draft ID if the provider returned a new one (e.g. Graph/IMAP
      // delete-then-create yields a different ID).
      if (result.success && result.data) {
        const newId =
          typeof result.data === "string"
            ? result.data
            : typeof result.data === "object" &&
                "draftId" in (result.data as Record<string, unknown>)
              ? (result.data as { draftId: string }).draftId
              : null;
        if (newId && newId !== state.draftId) {
          state.setDraftId(newId);
        }
      }
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

function runSaveNow(): Promise<void> {
  if (saveInFlight !== null) {
    return saveInFlight;
  }
  const savePromise = saveDraft().finally(() => {
    if (saveInFlight === savePromise) {
      saveInFlight = null;
    }
  });
  saveInFlight = savePromise;
  return savePromise;
}

function scheduleSave(): void {
  if (debounceTimer) clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    void runSaveNow();
  }, DEBOUNCE_MS);
}

function flushPendingSave(): void {
  if (!debounceTimer) return;
  clearTimeout(debounceTimer);
  debounceTimer = null;
  void runSaveNow();
}

function handlePageHide(): void {
  flushPendingSave();
}

function handleVisibilityChange(): void {
  if (document.visibilityState === "hidden") {
    flushPendingSave();
  }
}

/**
 * Start watching composerStore changes and auto-saving drafts.
 */
export function startAutoSave(accountId?: string): void {
  stopAutoSave();

  // Capture the account ID into the composer store at the time auto-save starts,
  // so drafts are always saved to the correct account even if the user switches.
  if (accountId) {
    const state = useComposerStore.getState();
    if (!state.accountId) {
      useComposerStore.setState({ accountId });
    }
  }

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

  window.addEventListener("pagehide", handlePageHide);
  document.addEventListener("visibilitychange", handleVisibilityChange);
}

/**
 * Stop auto-saving and clean up.
 */
export function stopAutoSave(): void {
  flushPendingSave();
  if (unsubscribe) {
    unsubscribe();
    unsubscribe = null;
  }
  window.removeEventListener("pagehide", handlePageHide);
  document.removeEventListener("visibilitychange", handleVisibilityChange);
}
