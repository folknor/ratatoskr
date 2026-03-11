import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  eventHandlers,
  mockInvoke,
  mockListen,
  mockListAccountBasicInfo,
  mockApplyCalendarSyncResult,
  mockUpsertDiscoveredCalendars,
  mockGetCalendarProvider,
  mockGetVisibleCalendars,
  mockQueueNewEmailNotification,
  mockApplySmartLabelsToNewMessageIds,
  mockCategorizeNewThreads,
} = vi.hoisted(() => ({
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  mockInvoke: vi.fn(),
  mockListen: vi.fn(),
  mockListAccountBasicInfo: vi.fn(),
  mockApplyCalendarSyncResult: vi.fn(),
  mockUpsertDiscoveredCalendars: vi.fn(),
  mockGetCalendarProvider: vi.fn(),
  mockGetVisibleCalendars: vi.fn(),
  mockQueueNewEmailNotification: vi.fn(),
  mockApplySmartLabelsToNewMessageIds: vi.fn(),
  mockCategorizeNewThreads: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockInvoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mockListen,
}));

vi.mock("@/services/notifications/notificationManager", () => ({
  queueNewEmailNotification: mockQueueNewEmailNotification,
}));

vi.mock("@/services/accounts/basicInfo", () => ({
  listAccountBasicInfo: mockListAccountBasicInfo,
}));

vi.mock("@/services/smartLabels/smartLabelManager", () => ({
  applySmartLabelsToNewMessageIds: mockApplySmartLabelsToNewMessageIds,
}));

vi.mock("@/services/ai/categorizationManager", () => ({
  categorizeNewThreads: mockCategorizeNewThreads,
}));

vi.mock("@/services/calendar/providerFactory", () => ({
  getCalendarProvider: mockGetCalendarProvider,
}));

vi.mock("@/services/calendar/persistence", () => ({
  applyCalendarSyncResult: mockApplyCalendarSyncResult,
  upsertDiscoveredCalendars: mockUpsertDiscoveredCalendars,
}));

vi.mock("@/services/db/calendars", () => ({
  getVisibleCalendars: mockGetVisibleCalendars,
}));

async function loadSyncManager() {
  return import("@/services/gmail/syncManager");
}

async function flushListenerSetup(): Promise<void> {
  for (let index = 0; index < 8; index += 1) {
    await Promise.resolve();
  }
}

async function flushAsyncWork(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function emitEvent(eventName: string, payload: unknown): void {
  const handler = eventHandlers.get(eventName);
  if (handler === undefined) {
    throw new Error(`No handler registered for ${eventName}`);
  }
  handler({ payload });
}

describe("syncManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.resetModules();
    eventHandlers.clear();
    mockApplySmartLabelsToNewMessageIds.mockResolvedValue(undefined);
    mockCategorizeNewThreads.mockResolvedValue(undefined);
    mockListAccountBasicInfo.mockResolvedValue([
      {
        id: "acc-1",
        email: "user1@example.com",
        displayName: "User One",
        avatarUrl: null,
        provider: "gmail_api",
        isActive: true,
      },
      {
        id: "acc-2",
        email: "user2@example.com",
        displayName: "User Two",
        avatarUrl: null,
        provider: "imap",
        isActive: true,
      },
      {
        id: "acc-caldav",
        email: "cal@example.com",
        displayName: "Calendar",
        avatarUrl: null,
        provider: "caldav",
        isActive: true,
      },
    ]);
    mockListen.mockImplementation(
      async (
        eventName: string,
        handler: (event: { payload: unknown }) => void,
      ) => {
        eventHandlers.set(eventName, handler);
        return () => {
          eventHandlers.delete(eventName);
        };
      },
    );
    mockGetVisibleCalendars.mockResolvedValue([]);
  });

  it("syncs one account through the rust queue command", async () => {
    const { syncAccount } = await loadSyncManager();

    await syncAccount("acc-1");

    expect(mockInvoke).toHaveBeenCalledWith("sync_run_accounts", {
      accountIds: ["acc-1"],
    });
  });

  it("starts background sync through rust and honors skipImmediateSync", async () => {
    const { startBackgroundSync } = await loadSyncManager();

    startBackgroundSync(["acc-1", "acc-2"], true);
    await flushAsyncWork();

    expect(mockInvoke).toHaveBeenNthCalledWith(1, "sync_stop_background");
    expect(mockInvoke).toHaveBeenNthCalledWith(2, "sync_start_background", {
      accountIds: ["acc-1", "acc-2"],
      skipImmediateSync: true,
    });
  });

  it("prepares full sync before running accounts", async () => {
    const { forceFullSync } = await loadSyncManager();

    await forceFullSync(["acc-1", "acc-2"]);

    expect(mockInvoke).toHaveBeenNthCalledWith(1, "sync_prepare_full_sync", {
      accountIds: ["acc-1", "acc-2"],
    });
    expect(mockInvoke).toHaveBeenNthCalledWith(2, "sync_run_accounts", {
      accountIds: ["acc-1", "acc-2"],
    });
  });

  it("prepares account resync before re-running that account", async () => {
    const { resyncAccount } = await loadSyncManager();

    await resyncAccount("acc-1");

    expect(mockInvoke).toHaveBeenNthCalledWith(
      1,
      "sync_prepare_account_resync",
      {
        accountId: "acc-1",
      },
    );
    expect(mockInvoke).toHaveBeenNthCalledWith(2, "sync_run_accounts", {
      accountIds: ["acc-1"],
    });
  });

  it("maps provider progress events to ui sync progress", async () => {
    const { onSyncStatus } = await loadSyncManager();
    const callback = vi.fn();

    const unsubscribe = onSyncStatus(callback);
    await flushListenerSetup();

    emitEvent("imap-sync-progress", {
      accountId: "acc-1",
      phase: "folders",
      current: 2,
      total: 5,
    });

    expect(callback).toHaveBeenCalledWith("acc-1", "syncing", {
      phase: "labels",
      current: 2,
      total: 5,
    });

    unsubscribe();
  });

  it("propagates sync-status errors including plain string errors", async () => {
    const { onSyncStatus } = await loadSyncManager();
    const callback = vi.fn();

    onSyncStatus(callback);
    await flushListenerSetup();

    emitEvent("sync-status", {
      accountId: "acc-1",
      provider: "gmail_api",
      status: "error",
      error: "authentication failed for user@test.com",
    });

    await Promise.resolve();

    expect(callback).toHaveBeenCalledWith(
      "acc-1",
      "error",
      undefined,
      "authentication failed for user@test.com",
    );
  });

  it("runs post-sync hooks from rust-provided completion data", async () => {
    const { onSyncStatus } = await loadSyncManager();
    const callback = vi.fn();

    onSyncStatus(callback);
    await flushListenerSetup();

    emitEvent("sync-status", {
      accountId: "acc-1",
      provider: "gmail_api",
      status: "done",
      newInboxMessageIds: ["m1", "m2"],
      affectedThreadIds: ["t1"],
      criteriaSmartLabelMatches: [{ threadId: "t1", labelIds: ["label-1"] }],
      notificationsToQueue: [
        {
          threadId: "t1",
          fromName: "Alice",
          fromAddress: "alice@example.com",
          subject: "Hello",
        },
      ],
      aiCategorizationCandidates: [{ threadId: "t1", subject: "Hello" }],
      aiSmartLabelThreads: [{ threadId: "t1", subject: "Hello" }],
      aiSmartLabelRules: [
        { id: "rule-1", name: "VIP", instructions: "Mark VIP" },
      ],
    });

    await flushAsyncWork();

    expect(mockApplySmartLabelsToNewMessageIds).toHaveBeenCalledWith(
      "acc-1",
      "gmail_api",
      ["m1", "m2"],
      [{ threadId: "t1", labelIds: ["label-1"] }],
      {
        threads: [{ threadId: "t1", subject: "Hello" }],
        rules: [{ id: "rule-1", name: "VIP", instructions: "Mark VIP" }],
      },
    );
    expect(mockQueueNewEmailNotification).toHaveBeenCalledWith(
      "Alice",
      "Hello",
      "t1",
      "acc-1",
      "alice@example.com",
    );
    expect(mockCategorizeNewThreads).toHaveBeenCalledWith("acc-1", [
      { threadId: "t1", subject: "Hello" },
    ]);
    expect(callback).toHaveBeenCalledWith("acc-1", "done");
  });

  it("runs calendar sync when rust marks it as needed", async () => {
    const { onSyncStatus } = await loadSyncManager();
    const callback = vi.fn();
    const provider = {
      type: "google",
      listCalendars: vi.fn().mockResolvedValue([
        {
          remoteId: "cal-1",
          displayName: "Primary",
          color: null,
          isPrimary: true,
        },
      ]),
      syncEvents: vi.fn().mockResolvedValue({
        calendars: [],
        events: [],
        deletedRemoteIds: [],
        nextSyncToken: "next-token",
      }),
    };

    mockGetCalendarProvider.mockResolvedValue(provider);
    mockGetVisibleCalendars.mockResolvedValue([
      {
        remote_id: "cal-1",
        display_name: "Primary",
        sync_token: "old-token",
      },
    ]);

    onSyncStatus(callback);
    await flushListenerSetup();

    emitEvent("sync-status", {
      accountId: "acc-1",
      provider: "gmail_api",
      status: "done",
      shouldSyncCalendar: true,
      newInboxMessageIds: [],
      affectedThreadIds: [],
    });

    await flushAsyncWork();
    await flushAsyncWork();

    expect(mockGetCalendarProvider).toHaveBeenCalledWith("acc-1");
    expect(mockUpsertDiscoveredCalendars).toHaveBeenCalledWith(
      "acc-1",
      "google",
      [
        {
          remoteId: "cal-1",
          displayName: "Primary",
          color: null,
          isPrimary: true,
        },
      ],
    );
    expect(provider.syncEvents).toHaveBeenCalledWith("cal-1", "old-token");
    expect(mockApplyCalendarSyncResult).toHaveBeenCalledWith("acc-1", "cal-1", {
      calendars: [],
      events: [],
      deletedRemoteIds: [],
      nextSyncToken: "next-token",
    });
  });

  it("syncs standalone caldav accounts directly in ts", async () => {
    const { onSyncStatus, syncAccount } = await loadSyncManager();
    const callback = vi.fn();
    const provider = {
      type: "caldav",
      listCalendars: vi.fn().mockResolvedValue([
        {
          remoteId: "cal-1",
          displayName: "Personal",
          color: null,
          isPrimary: false,
        },
      ]),
      syncEvents: vi.fn().mockResolvedValue({
        calendars: [],
        events: [],
        deletedRemoteIds: [],
        nextSyncToken: "next-token",
      }),
    };

    mockGetCalendarProvider.mockResolvedValue(provider);
    mockGetVisibleCalendars.mockResolvedValue([
      {
        remote_id: "cal-1",
        display_name: "Personal",
        sync_token: null,
      },
    ]);

    onSyncStatus(callback);
    await syncAccount("acc-caldav");

    expect(mockGetCalendarProvider).toHaveBeenCalledWith("acc-caldav");
    expect(provider.syncEvents).toHaveBeenCalledWith("cal-1", undefined);
    expect(mockApplyCalendarSyncResult).toHaveBeenCalledWith(
      "acc-caldav",
      "cal-1",
      {
        calendars: [],
        events: [],
        deletedRemoteIds: [],
        nextSyncToken: "next-token",
      },
    );
    expect(callback).toHaveBeenCalledWith("acc-caldav", "done");
    expect(mockInvoke).not.toHaveBeenCalledWith("sync_run_accounts", {
      accountIds: ["acc-caldav"],
    });

    const applyOrder = mockApplyCalendarSyncResult.mock.invocationCallOrder[0];
    const doneOrder = callback.mock.invocationCallOrder.at(-1);
    expect(applyOrder).toBeLessThan(doneOrder ?? 0);
  });
});
