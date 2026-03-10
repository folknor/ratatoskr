import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock dependencies
vi.mock("@/stores/syncStateStore", () => ({
  useSyncStateStore: {
    getState: vi.fn(() => ({ isOnline: true })),
  },
}));

vi.mock("@/stores/threadStore", () => ({
  useThreadStore: {
    getState: vi.fn(() => ({
      updateThread: vi.fn(),
      removeThread: vi.fn(),
    })),
  },
}));

vi.mock("@/services/email/providerFactory", () => ({
  getEmailProvider: vi.fn(),
}));

vi.mock("@/services/db/accounts", () => ({
  getAccount: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/core/rustDb", () => ({
  emailActionArchive: vi.fn(() => Promise.resolve()),
  emailActionTrash: vi.fn(() => Promise.resolve()),
  emailActionPermanentDelete: vi.fn(() => Promise.resolve()),
  emailActionSpam: vi.fn(() => Promise.resolve()),
  emailActionMarkRead: vi.fn(() => Promise.resolve()),
  emailActionStar: vi.fn(() => Promise.resolve()),
  emailActionSnooze: vi.fn(() => Promise.resolve()),
  emailActionPin: vi.fn(() => Promise.resolve()),
  emailActionUnpin: vi.fn(() => Promise.resolve()),
  emailActionMute: vi.fn(() => Promise.resolve()),
  emailActionUnmute: vi.fn(() => Promise.resolve()),
  emailActionAddLabel: vi.fn(() => Promise.resolve()),
  emailActionRemoveLabel: vi.fn(() => Promise.resolve()),
  emailActionMoveToFolder: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/services/db/pendingOperations", () => ({
  enqueuePendingOperation: vi.fn(() => Promise.resolve("op-1")),
}));

vi.mock("@/router/navigate", () => ({
  navigateToThread: vi.fn(),
  getSelectedThreadId: vi.fn(() => null),
}));

import { invoke } from "@tauri-apps/api/core";
import { enqueuePendingOperation } from "@/services/db/pendingOperations";
import { getSelectedThreadId, navigateToThread } from "@/router/navigate";
import { getEmailProvider } from "@/services/email/providerFactory";
import { getAccount } from "@/services/db/accounts";
import { useThreadStore } from "@/stores/threadStore";
import { useSyncStateStore } from "@/stores/syncStateStore";
import {
  createMockEmailProvider,
  createMockImapAccount,
  createMockGmailAccount,
  createMockThreadStoreState,
  createMockUIStoreState,
} from "@/test/mocks";
import {
  archiveThread,
  executeEmailAction,
  markThreadRead,
  moveThread,
  permanentDeleteThread,
  spamThread,
  starThread,
  trashThread,
} from "./emailActions";

const mockProvider = createMockEmailProvider();

const mockUpdateThread = vi.fn();
const mockRemoveThread = vi.fn();

describe("emailActions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getEmailProvider).mockResolvedValue(mockProvider as never);
    vi.mocked(getAccount).mockResolvedValue(
      createMockImapAccount() as never,
    );
    vi.mocked(useSyncStateStore.getState).mockReturnValue(
      createMockUIStoreState() as never,
    );
    vi.mocked(useThreadStore.getState).mockReturnValue(
      createMockThreadStoreState({
        updateThread: mockUpdateThread,
        removeThread: mockRemoveThread,
      }) as never,
    );
  });

  describe("IMAP online execution (via provider)", () => {
    it("archives a thread via provider", async () => {
      const result = await archiveThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(result.queued).toBeUndefined();
      expect(mockRemoveThread).toHaveBeenCalledWith("t1");
      expect(mockProvider.archive).toHaveBeenCalledWith("t1", ["m1"]);
    });

    it("trashes a thread via provider", async () => {
      const result = await trashThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(mockProvider.trash).toHaveBeenCalledWith("t1", ["m1"]);
    });

    it("stars a thread via provider", async () => {
      const result = await starThread("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(true);
      expect(mockUpdateThread).toHaveBeenCalledWith("t1", { isStarred: true });
      expect(mockProvider.star).toHaveBeenCalledWith("t1", ["m1"], true);
    });

    it("marks thread read via provider", async () => {
      const result = await markThreadRead("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(true);
      expect(mockUpdateThread).toHaveBeenCalledWith("t1", { isRead: true });
      expect(mockProvider.markRead).toHaveBeenCalledWith("t1", ["m1"], true);
    });

    it("reports spam via provider", async () => {
      const result = await spamThread("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(true);
      expect(mockRemoveThread).toHaveBeenCalledWith("t1");
      expect(mockProvider.spam).toHaveBeenCalledWith("t1", ["m1"], true);
    });
  });

  describe("Gmail online execution (via Rust invoke)", () => {
    beforeEach(() => {
      vi.mocked(getAccount).mockResolvedValue(
        createMockGmailAccount() as never,
      );
    });

    it("archives a thread via gmail_modify_thread", async () => {
      const result = await archiveThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: [],
        removeLabels: ["INBOX"],
      });
      expect(mockProvider.archive).not.toHaveBeenCalled();
    });

    it("trashes a thread via gmail_modify_thread", async () => {
      const result = await trashThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: ["TRASH"],
        removeLabels: ["INBOX"],
      });
    });

    it("permanently deletes a thread via gmail_delete_thread", async () => {
      const result = await permanentDeleteThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_delete_thread", {
        accountId: "acct-1",
        threadId: "t1",
      });
    });

    it("marks thread read via gmail_modify_thread", async () => {
      const result = await markThreadRead("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: [],
        removeLabels: ["UNREAD"],
      });
    });

    it("marks thread unread via gmail_modify_thread", async () => {
      const result = await markThreadRead("acct-1", "t1", ["m1"], false);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: ["UNREAD"],
        removeLabels: [],
      });
    });

    it("stars a thread via gmail_modify_thread", async () => {
      const result = await starThread("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: ["STARRED"],
        removeLabels: [],
      });
    });

    it("unstars a thread via gmail_modify_thread", async () => {
      const result = await starThread("acct-1", "t1", ["m1"], false);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: [],
        removeLabels: ["STARRED"],
      });
    });

    it("reports spam via gmail_modify_thread", async () => {
      const result = await spamThread("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: ["SPAM"],
        removeLabels: ["INBOX"],
      });
    });

    it("sends a message via gmail_send_email", async () => {
      const result = await executeEmailAction("acct-1", {
        type: "sendMessage",
        rawBase64Url: "base64data",
        threadId: "t1",
      });
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_send_email", {
        accountId: "acct-1",
        raw: "base64data",
        threadId: "t1",
      });
    });

    it("creates a draft via gmail_create_draft", async () => {
      const result = await executeEmailAction("acct-1", {
        type: "createDraft",
        rawBase64Url: "base64data",
      });
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_create_draft", {
        accountId: "acct-1",
        raw: "base64data",
        threadId: null,
      });
    });

    it("updates a draft via gmail_update_draft", async () => {
      const result = await executeEmailAction("acct-1", {
        type: "updateDraft",
        draftId: "d1",
        rawBase64Url: "base64data",
        threadId: "t1",
      });
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_update_draft", {
        accountId: "acct-1",
        draftId: "d1",
        raw: "base64data",
        threadId: "t1",
      });
    });

    it("deletes a draft via gmail_delete_draft", async () => {
      const result = await executeEmailAction("acct-1", {
        type: "deleteDraft",
        draftId: "d1",
      });
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_delete_draft", {
        accountId: "acct-1",
        draftId: "d1",
      });
    });

    it("moves to folder via gmail_modify_thread", async () => {
      const result = await moveThread("acct-1", "t1", ["m1"], "Label_123");
      expect(result.success).toBe(true);
      expect(invoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "acct-1",
        threadId: "t1",
        addLabels: ["Label_123"],
        removeLabels: [],
      });
    });
  });

  describe("offline queueing", () => {
    beforeEach(() => {
      vi.mocked(useSyncStateStore.getState).mockReturnValue({
        isOnline: false,
      } as never);
    });

    it("queues archive when offline", async () => {
      const result = await archiveThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(result.queued).toBe(true);
      expect(mockProvider.archive).not.toHaveBeenCalled();
      expect(invoke).not.toHaveBeenCalled();
      expect(enqueuePendingOperation).toHaveBeenCalledWith(
        "acct-1",
        "archive",
        "t1",
        expect.objectContaining({ threadId: "t1", messageIds: ["m1"] }),
      );
    });

    it("still applies optimistic UI update when offline", async () => {
      await starThread("acct-1", "t1", ["m1"], true);
      expect(mockUpdateThread).toHaveBeenCalledWith("t1", { isStarred: true });
    });
  });

  describe("network error → queue fallback", () => {
    it("queues on retryable network error (IMAP)", async () => {
      vi.mocked(useSyncStateStore.getState).mockReturnValue({
        isOnline: true,
      } as never);
      mockProvider.archive.mockRejectedValueOnce(new Error("Failed to fetch"));

      const result = await archiveThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(result.queued).toBe(true);
      expect(enqueuePendingOperation).toHaveBeenCalled();
    });

    it("queues on retryable network error (Gmail)", async () => {
      vi.mocked(getAccount).mockResolvedValue(
        createMockGmailAccount() as never,
      );
      vi.mocked(useSyncStateStore.getState).mockReturnValue({
        isOnline: true,
      } as never);
      vi.mocked(invoke).mockRejectedValueOnce(new Error("Failed to fetch"));

      const result = await archiveThread("acct-1", "t1", ["m1"]);
      expect(result.success).toBe(true);
      expect(result.queued).toBe(true);
      expect(enqueuePendingOperation).toHaveBeenCalled();
    });
  });

  describe("permanent error → revert", () => {
    it("reverts star on permanent error (IMAP)", async () => {
      vi.mocked(useSyncStateStore.getState).mockReturnValue({
        isOnline: true,
      } as never);
      mockProvider.star.mockRejectedValueOnce(new Error("Invalid request"));

      const result = await starThread("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(false);
      expect(result.error).toBeTruthy();
      // Revert: set starred to false
      expect(mockUpdateThread).toHaveBeenCalledWith("t1", { isStarred: false });
    });

    it("reverts markRead on permanent error (IMAP)", async () => {
      vi.mocked(useSyncStateStore.getState).mockReturnValue({
        isOnline: true,
      } as never);
      mockProvider.markRead.mockRejectedValueOnce(new Error("Bad request"));

      const result = await markThreadRead("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(false);
      // Revert: set read to false
      expect(mockUpdateThread).toHaveBeenCalledWith("t1", { isRead: false });
    });

    it("reverts star on permanent error (Gmail)", async () => {
      vi.mocked(getAccount).mockResolvedValue(
        createMockGmailAccount() as never,
      );
      vi.mocked(useSyncStateStore.getState).mockReturnValue({
        isOnline: true,
      } as never);
      vi.mocked(invoke).mockRejectedValueOnce(new Error("Invalid request"));

      const result = await starThread("acct-1", "t1", ["m1"], true);
      expect(result.success).toBe(false);
      expect(result.error).toBeTruthy();
      expect(mockUpdateThread).toHaveBeenCalledWith("t1", { isStarred: false });
    });
  });

  describe("auto-advance after removal", () => {
    const threads = [{ id: "t1" }, { id: "t2" }, { id: "t3" }];

    it("navigates to next thread when archiving the viewed thread", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t2");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await archiveThread("acct-1", "t2", ["m1"]);
      expect(navigateToThread).toHaveBeenCalledWith("t3");
    });

    it("navigates to previous thread when archiving the last thread", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t3");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await archiveThread("acct-1", "t3", ["m1"]);
      expect(navigateToThread).toHaveBeenCalledWith("t2");
    });

    it("does not navigate when archiving a non-viewed thread", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t1");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await archiveThread("acct-1", "t2", ["m1"]);
      expect(navigateToThread).not.toHaveBeenCalled();
    });

    it("does not navigate when archiving the only thread", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t1");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads: [{ id: "t1" }],
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await archiveThread("acct-1", "t1", ["m1"]);
      expect(navigateToThread).not.toHaveBeenCalled();
    });

    it("navigates on trash action", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t1");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await trashThread("acct-1", "t1", ["m1"]);
      expect(navigateToThread).toHaveBeenCalledWith("t2");
    });

    it("navigates on spam action", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t1");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await spamThread("acct-1", "t1", ["m1"], true);
      expect(navigateToThread).toHaveBeenCalledWith("t2");
    });

    it("navigates on permanentDelete action", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t2");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await permanentDeleteThread("acct-1", "t2", ["m1"]);
      expect(navigateToThread).toHaveBeenCalledWith("t3");
    });

    it("navigates on moveToFolder action", async () => {
      vi.mocked(getSelectedThreadId).mockReturnValue("t2");
      vi.mocked(useThreadStore.getState).mockReturnValue(
        createMockThreadStoreState({
          threads,
          updateThread: mockUpdateThread,
          removeThread: mockRemoveThread,
        }) as never,
      );

      await moveThread("acct-1", "t2", ["m1"], "Archive");
      expect(navigateToThread).toHaveBeenCalledWith("t3");
    });
  });

  describe("executeEmailAction with draft actions (IMAP)", () => {
    it("sends a message via provider", async () => {
      const result = await executeEmailAction("acct-1", {
        type: "sendMessage",
        rawBase64Url: "base64data",
        threadId: "t1",
      });
      expect(result.success).toBe(true);
      expect(mockProvider.sendMessage).toHaveBeenCalledWith("base64data", "t1");
    });

    it("creates a draft via provider", async () => {
      const result = await executeEmailAction("acct-1", {
        type: "createDraft",
        rawBase64Url: "base64data",
      });
      expect(result.success).toBe(true);
      expect(mockProvider.createDraft).toHaveBeenCalledWith(
        "base64data",
        undefined,
      );
    });
  });
});
