import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock all dependencies before importing the module under test
vi.mock("./sync", () => ({
  // SyncProgress type is imported as a type-only import, no mock needed
}));
vi.mock("../db/accounts", () => ({
  getAccount: vi.fn(),
  clearAccountHistoryId: vi.fn(),
}));
vi.mock("../db/settings", () => ({
  getSetting: vi.fn().mockResolvedValue("365"),
}));
vi.mock("../db/threads", () => ({
  getThreadCountForAccount: vi.fn(),
  deleteAllThreadsForAccount: vi.fn(),
  getMutedThreadIds: vi.fn().mockResolvedValue(new Set()),
}));
vi.mock("../db/messages", () => ({
  deleteAllMessagesForAccount: vi.fn(),
  getMessagesByIds: vi.fn().mockResolvedValue([]),
}));
vi.mock("../imap/imapSync", () => ({
  imapInitialSync: vi.fn(),
  imapDeltaSync: vi.fn(),
}));
vi.mock("../db/folderSyncState", () => ({
  clearAllFolderSyncStates: vi.fn(),
}));
vi.mock("../oauth/oauthTokenManager", () => ({
  ensureFreshToken: vi.fn(),
}));
vi.mock("../calendar/providerFactory", () => ({
  hasCalendarSupport: vi.fn().mockResolvedValue(false),
  getCalendarProvider: vi.fn(),
}));
vi.mock("../db/calendars", () => ({
  getVisibleCalendars: vi.fn().mockResolvedValue([]),
  upsertCalendar: vi.fn(),
  updateCalendarSyncToken: vi.fn(),
}));
vi.mock("../db/calendarEvents", () => ({
  upsertCalendarEvent: vi.fn(),
  deleteEventByRemoteId: vi.fn(),
}));
vi.mock("@/core/rustDb", () => ({
  syncImapInitial: vi.fn().mockResolvedValue({
    storedCount: 0,
    threadCount: 0,
    newInboxMessageIds: [],
    affectedThreadIds: [],
  }),
  syncImapDelta: vi.fn().mockResolvedValue({
    storedCount: 0,
    threadCount: 0,
    newInboxMessageIds: [],
    affectedThreadIds: [],
  }),
  syncGmailInitial: vi.fn().mockResolvedValue(undefined),
  syncGmailDelta: vi
    .fn()
    .mockResolvedValue({ newInboxMessageIds: [], affectedThreadIds: [] }),
}));
vi.mock("@/services/filters/filterEngine", () => ({
  applyFiltersToNewMessageIds: vi.fn(),
}));
vi.mock("@/services/smartLabels/smartLabelManager", () => ({
  applySmartLabelsToNewMessageIds: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/services/ai/categorizationManager", () => ({
  categorizeNewThreads: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@/services/db/threadCategories", () => ({
  getThreadCategory: vi.fn(),
}));
vi.mock("@/services/db/notificationVips", () => ({
  getVipSenders: vi.fn().mockResolvedValue(new Set()),
}));
vi.mock("@/services/notifications/notificationManager", () => ({
  queueNewEmailNotification: vi.fn(),
  shouldNotifyForMessage: vi.fn().mockReturnValue(false),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

import { syncGmailDelta, syncGmailInitial } from "@/core/rustDb";
import { getAccount } from "../db/accounts";
// Import after mocks
import {
  onSyncStatus,
  startBackgroundSync,
  stopBackgroundSync,
  syncAccount,
  triggerSync,
} from "./syncManager";

const mockGetAccount = vi.mocked(getAccount);
const mockSyncGmailInitial = vi.mocked(syncGmailInitial);
const mockSyncGmailDelta = vi.mocked(syncGmailDelta);

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function makeGmailAccount(id: string, historyId: string | null = null) {
  return {
    id,
    email: `${id}@gmail.com`,
    display_name: id,
    avatar_url: null,
    is_active: 1,
    provider: "gmail_api" as const,
    history_id: historyId,
    refresh_token: "tok",
    access_token: "tok",
    token_expiry: Date.now() + 60_000,
    client_id: "cid",
    client_secret: null,
    created_at: new Date().toISOString(),
    imap_host: null,
    imap_port: null,
    imap_security: null,
    smtp_host: null,
    smtp_port: null,
    smtp_security: null,
    auth_method: null,
    imap_password: null,
    imap_username: null,
  };
}

describe("syncManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    stopBackgroundSync();
    mockSyncGmailInitial.mockResolvedValue(undefined);
    mockSyncGmailDelta.mockResolvedValue({
      newInboxMessageIds: [],
      affectedThreadIds: [],
    });
  });

  afterEach(() => {
    stopBackgroundSync();
  });

  describe("syncAccount", () => {
    it("runs initial sync for an account without history_id", async () => {
      mockGetAccount.mockResolvedValue(makeGmailAccount("a1"));

      await syncAccount("a1");

      expect(mockSyncGmailInitial).toHaveBeenCalledTimes(1);
      expect(mockSyncGmailInitial).toHaveBeenCalledWith("a1", 365);
      expect(mockSyncGmailDelta).not.toHaveBeenCalled();
    });

    it("runs delta sync for an account with history_id", async () => {
      mockGetAccount.mockResolvedValue(makeGmailAccount("a1", "12345"));

      await syncAccount("a1");

      expect(mockSyncGmailDelta).toHaveBeenCalledTimes(1);
      expect(mockSyncGmailDelta).toHaveBeenCalledWith("a1");
      expect(mockSyncGmailInitial).not.toHaveBeenCalled();
    });

    it("queues a second account while sync is in progress", async () => {
      const a1 = makeGmailAccount("a1", "100");
      const a2 = makeGmailAccount("a2", "200");

      mockGetAccount.mockImplementation(async (id: string) => {
        if (id === "a1") return a1;
        if (id === "a2") return a2;
        return null;
      });

      // Make first sync slow
      const barrier = new Promise<void>((r) => {
        // Resolve after 50ms
        setTimeout(r, 50);
      });
      let firstCall = true;
      mockSyncGmailDelta.mockImplementation(() => {
        if (firstCall) {
          firstCall = false;
          return barrier.then(() => ({
            newInboxMessageIds: [],
            affectedThreadIds: [],
          }));
        }
        return Promise.resolve({
          newInboxMessageIds: [],
          affectedThreadIds: [],
        });
      });

      const first = syncAccount("a1");
      // a2 will be queued since a1 is in progress
      const second = syncAccount("a2");

      await first;
      await second;

      // Both accounts synced (a1 directly, a2 via queue drain)
      expect(mockSyncGmailDelta).toHaveBeenCalledTimes(2);
    });
  });

  describe("startBackgroundSync", () => {
    it("triggers an immediate sync by default", async () => {
      mockGetAccount.mockResolvedValue(makeGmailAccount("a1", "100"));

      startBackgroundSync(["a1"]);

      // Wait for async sync chain to complete
      await wait(50);

      expect(mockSyncGmailDelta).toHaveBeenCalledTimes(1);
    });

    it("skips immediate sync when skipImmediateSync is true", async () => {
      mockGetAccount.mockResolvedValue(makeGmailAccount("a1", "100"));

      startBackgroundSync(["a1"], true);

      // Wait — no sync should have fired (next interval is 15s away)
      await wait(50);

      expect(mockSyncGmailDelta).not.toHaveBeenCalled();
      expect(mockGetAccount).not.toHaveBeenCalled();
    });
  });

  describe("new account sync priority", () => {
    it("new account syncs immediately when background sync skips immediate run", async () => {
      const existingAccount = makeGmailAccount("existing", "100");
      const newAccount = makeGmailAccount("new-acc");

      mockGetAccount.mockImplementation(async (id: string) => {
        if (id === "existing") return existingAccount;
        if (id === "new-acc") return newAccount;
        return null;
      });

      // Simulate the fix: sync new account first, then start background with skipImmediate
      const syncPromise = syncAccount("new-acc");
      startBackgroundSync(["existing", "new-acc"], true);

      await syncPromise;

      // The new account got an initial sync immediately
      expect(mockSyncGmailInitial).toHaveBeenCalledTimes(1);
      // No delta sync ran (background timer hasn't fired)
      expect(mockSyncGmailDelta).not.toHaveBeenCalled();
    });

    it("without the fix, new account sync would be blocked by existing account sync", async () => {
      const existingAccount = makeGmailAccount("existing", "100");
      const newAccount = makeGmailAccount("new-acc");

      // Track the order of sync calls
      const syncOrder: string[] = [];

      mockGetAccount.mockImplementation(async (id: string) => {
        if (id === "existing") return existingAccount;
        if (id === "new-acc") return newAccount;
        return null;
      });

      mockSyncGmailDelta.mockImplementation(async () => {
        syncOrder.push("delta-existing");
        return { newInboxMessageIds: [], affectedThreadIds: [] };
      });
      mockSyncGmailInitial.mockImplementation(async () => {
        syncOrder.push("initial-new");
      });

      // Old behavior: startBackgroundSync first (with immediate sync), then syncAccount
      // This would queue new-acc behind existing account's delta sync
      startBackgroundSync(["existing", "new-acc"]);

      // Wait for both to complete
      await wait(50);

      // existing account's delta sync ran BEFORE new account's initial sync
      expect(syncOrder).toEqual(["delta-existing", "initial-new"]);
    });
  });

  describe("triggerSync", () => {
    it("syncs all provided accounts", async () => {
      const a1 = makeGmailAccount("a1", "100");
      const a2 = makeGmailAccount("a2", "200");

      mockGetAccount.mockImplementation(async (id: string) => {
        if (id === "a1") return a1;
        if (id === "a2") return a2;
        return null;
      });

      await triggerSync(["a1", "a2"]);

      expect(mockSyncGmailDelta).toHaveBeenCalledTimes(2);
    });
  });

  describe("error coercion", () => {
    it("propagates plain string errors from Tauri IPC (not 'Unknown error')", async () => {
      const account = makeGmailAccount("a1", "100");
      mockGetAccount.mockResolvedValue(account);
      // Tauri IPC rejects with a plain string, not an Error instance
      mockSyncGmailDelta.mockRejectedValue(
        "authentication failed for user@test.com",
      );

      const errors: string[] = [];
      const unsub = onSyncStatus((_id, status, _progress, error) => {
        if (status === "error" && error) errors.push(error);
      });

      await syncAccount("a1");
      unsub();

      expect(errors).toHaveLength(1);
      expect(errors[0]).toBe("authentication failed for user@test.com");
      expect(errors[0]).not.toBe("Unknown error");
    });

    it("handles null/undefined errors gracefully", async () => {
      const account = makeGmailAccount("a1", "100");
      mockGetAccount.mockResolvedValue(account);
      mockSyncGmailDelta.mockRejectedValue(null);

      const errors: string[] = [];
      const unsub = onSyncStatus((_id, status, _progress, error) => {
        if (status === "error" && error) errors.push(error);
      });

      await syncAccount("a1");
      unsub();

      expect(errors).toHaveLength(1);
      expect(errors[0]).toBe("Unknown error");
    });
  });
});
