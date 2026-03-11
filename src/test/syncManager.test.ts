import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  eventHandlers,
  mockInvoke,
  mockListen,
  mockListAccountBasicInfo,
} = vi.hoisted(() => ({
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  mockInvoke: vi.fn(),
  mockListen: vi.fn(),
  mockListAccountBasicInfo: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockInvoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mockListen,
}));

vi.mock("@/services/accounts/basicInfo", () => ({
  listAccountBasicInfo: mockListAccountBasicInfo,
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

    expect(mockInvoke).toHaveBeenNthCalledWith(1, "provider_prepare_full_sync", {
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
      "provider_prepare_account_resync",
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

  it("maps fallback progress events to ui sync progress", async () => {
    const { onSyncStatus } = await loadSyncManager();
    const callback = vi.fn();

    const unsubscribe = onSyncStatus(callback);
    await flushListenerSetup();

    emitEvent("jmap-sync-progress", {
      accountId: "acc-1",
      phase: "fallback",
      current: 0,
      total: 1,
    });

    expect(callback).toHaveBeenCalledWith("acc-1", "syncing", {
      phase: "fallback",
      current: 0,
      total: 1,
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
      result: {
        newInboxMessageIds: ["m1", "m2"],
        affectedThreadIds: ["t1"],
        criteriaSmartLabelMatches: [{ threadId: "t1", labelIds: ["label-1"] }],
      },
    });

    await flushAsyncWork();
    expect(callback).toHaveBeenCalledWith("acc-1", "done");
  });

  it("marks sync done without ts calendar follow-up work", async () => {
    const { onSyncStatus } = await loadSyncManager();
    const callback = vi.fn();

    onSyncStatus(callback);
    await flushListenerSetup();

    emitEvent("sync-status", {
      accountId: "acc-1",
      provider: "gmail_api",
      status: "done",
      result: {
        newInboxMessageIds: [],
        affectedThreadIds: [],
        criteriaSmartLabelMatches: [],
      },
    });

    await flushAsyncWork();
    expect(callback).toHaveBeenCalledWith("acc-1", "done");
  });

  it("syncs standalone caldav accounts directly in ts", async () => {
    const { onSyncStatus, syncAccount } = await loadSyncManager();
    const callback = vi.fn();

    onSyncStatus(callback);
    await syncAccount("acc-caldav");

    expect(mockInvoke).toHaveBeenCalledWith("calendar_sync_account", {
      accountId: "acc-caldav",
    });
    expect(callback).toHaveBeenCalledWith("acc-caldav", "done");
    expect(mockInvoke).not.toHaveBeenCalledWith("sync_run_accounts", {
      accountIds: ["acc-caldav"],
    });

    const applyIndex = mockInvoke.mock.calls.findIndex(
      ([command]) => command === "calendar_sync_account",
    );
    const applyOrder =
      applyIndex >= 0 ? mockInvoke.mock.invocationCallOrder[applyIndex] : -1;
    const doneOrder = callback.mock.invocationCallOrder.at(-1);
    expect(applyOrder).toBeGreaterThanOrEqual(0);
    expect(applyOrder).toBeLessThan(doneOrder ?? 0);
  });
});
