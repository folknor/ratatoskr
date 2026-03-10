import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";

export type ComposerMode = "new" | "reply" | "replyAll" | "forward";
export type ComposerViewMode = "modal" | "fullpage";

export interface ComposerAttachment {
  id: string;
  file: File;
  filename: string;
  mimeType: string;
  size: number;
  content: string; // base64
}

export interface ComposerState {
  isOpen: boolean;
  mode: ComposerMode;
  /** The account ID this composer session belongs to, captured at open time. */
  accountId: string | null;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  bodyHtml: string;
  threadId: string | null;
  inReplyToMessageId: string | null;
  showCcBcc: boolean;
  draftId: string | null;
  undoSendTimer: ReturnType<typeof setTimeout> | null;
  undoSendVisible: boolean;
  undoSendDelaySeconds: number;
  attachments: ComposerAttachment[];
  lastSavedAt: number | null;
  isSaving: boolean;
  fromEmail: string | null;
  viewMode: ComposerViewMode;
  signatureHtml: string;
  signatureId: string | null;

  openComposer: (opts?: {
    mode?: ComposerMode;
    accountId?: string | null;
    to?: string[];
    cc?: string[];
    bcc?: string[];
    subject?: string;
    bodyHtml?: string;
    threadId?: string | null;
    inReplyToMessageId?: string | null;
    draftId?: string | null;
  }) => void;
  closeComposer: () => void;
  setTo: (to: string[]) => void;
  setCc: (cc: string[]) => void;
  setBcc: (bcc: string[]) => void;
  setSubject: (subject: string) => void;
  setBodyHtml: (bodyHtml: string) => void;
  setShowCcBcc: (show: boolean) => void;
  setDraftId: (id: string | null) => void;
  setUndoSendTimer: (timer: ReturnType<typeof setTimeout> | null) => void;
  setUndoSendVisible: (visible: boolean, delaySeconds?: number) => void;
  addAttachment: (attachment: ComposerAttachment) => void;
  removeAttachment: (id: string) => void;
  clearAttachments: () => void;
  setLastSavedAt: (ts: number | null) => void;
  setIsSaving: (saving: boolean) => void;
  setFromEmail: (email: string | null) => void;
  setViewMode: (mode: ComposerViewMode) => void;
  setSignatureHtml: (html: string) => void;
  setSignatureId: (id: string | null) => void;
}

export const useComposerStore: UseBoundStore<StoreApi<ComposerState>> =
  create<ComposerState>((set) => ({
    isOpen: false,
    mode: "new",
    accountId: null,
    to: [],
    cc: [],
    bcc: [],
    subject: "",
    bodyHtml: "",
    threadId: null,
    inReplyToMessageId: null,
    showCcBcc: false,
    draftId: null,
    undoSendTimer: null,
    undoSendVisible: false,
    undoSendDelaySeconds: 5,
    attachments: [],
    viewMode: "modal",
    fromEmail: null,
    lastSavedAt: null,
    isSaving: false,
    signatureHtml: "",
    signatureId: null,

    openComposer: (opts?: {
      mode?: ComposerMode;
      accountId?: string | null;
      to?: string[];
      cc?: string[];
      bcc?: string[];
      subject?: string;
      bodyHtml?: string;
      threadId?: string | null;
      inReplyToMessageId?: string | null;
      draftId?: string | null;
    }) =>
      set({
        isOpen: true,
        mode: opts?.mode ?? "new",
        accountId: opts?.accountId ?? null,
        to: opts?.to ?? [],
        cc: opts?.cc ?? [],
        bcc: opts?.bcc ?? [],
        subject: opts?.subject ?? "",
        bodyHtml: opts?.bodyHtml ?? "",
        threadId: opts?.threadId ?? null,
        inReplyToMessageId: opts?.inReplyToMessageId ?? null,
        showCcBcc: (opts?.cc?.length ?? 0) > 0 || (opts?.bcc?.length ?? 0) > 0,
        draftId: opts?.draftId ?? null,
        viewMode: "modal",
        fromEmail: null,
        attachments: [],
        lastSavedAt: null,
        isSaving: false,
        signatureHtml: "",
        signatureId: null,
      }),
    closeComposer: () =>
      set({
        isOpen: false,
        mode: "new",
        accountId: null,
        to: [],
        cc: [],
        bcc: [],
        subject: "",
        bodyHtml: "",
        threadId: null,
        inReplyToMessageId: null,
        showCcBcc: false,
        draftId: null,
        viewMode: "modal",
        fromEmail: null,
        attachments: [],
        lastSavedAt: null,
        isSaving: false,
        signatureHtml: "",
        signatureId: null,
      }),
    setTo: (to: string[]) => set({ to }),
    setCc: (cc: string[]) => set({ cc }),
    setBcc: (bcc: string[]) => set({ bcc }),
    setSubject: (subject: string) => set({ subject }),
    setBodyHtml: (bodyHtml: string) => set({ bodyHtml }),
    setShowCcBcc: (showCcBcc: boolean) => set({ showCcBcc }),
    setDraftId: (draftId: string | null) => set({ draftId }),
    setUndoSendTimer: (undoSendTimer: ReturnType<typeof setTimeout> | null) =>
      set({ undoSendTimer }),
    setUndoSendVisible: (undoSendVisible: boolean, delaySeconds?: number) =>
      set({
        undoSendVisible,
        ...(delaySeconds != null ? { undoSendDelaySeconds: delaySeconds } : {}),
      }),
    addAttachment: (attachment: ComposerAttachment) =>
      set((state) => ({ attachments: [...state.attachments, attachment] })),
    removeAttachment: (id: string) =>
      set((state) => ({
        attachments: state.attachments.filter((a) => a.id !== id),
      })),
    clearAttachments: () => set({ attachments: [] }),
    setLastSavedAt: (lastSavedAt: number | null) => set({ lastSavedAt }),
    setIsSaving: (isSaving: boolean) => set({ isSaving }),
    setFromEmail: (fromEmail: string | null) => set({ fromEmail }),
    setViewMode: (viewMode: ComposerViewMode) => set({ viewMode }),
    setSignatureHtml: (signatureHtml: string) => set({ signatureHtml }),
    setSignatureId: (signatureId: string | null) => set({ signatureId }),
  }));
