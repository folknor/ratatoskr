import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@/stores/syncStateStore", () => ({
  useSyncStateStore: {
    getState: vi.fn(() => ({ isOnline: true })),
  },
}));

vi.mock("../db/settings", () => ({
  getSetting: vi.fn(() => Promise.resolve("500")),
}));

const mockFetchAttachment = vi.fn();
vi.mock("../email/providerFactory", () => ({
  getEmailProvider: vi.fn(() =>
    Promise.resolve({ fetchAttachment: mockFetchAttachment }),
  ),
}));

vi.mock("./cacheManager", () => ({
  cacheAttachment: vi.fn(),
}));

let lastRunPromise: Promise<void> = Promise.resolve();
vi.mock("../backgroundCheckers", () => ({
  createBackgroundChecker: vi.fn((_name: string, fn: () => Promise<void>) => ({
    start: () => {
      lastRunPromise = fn();
    },
    stop: vi.fn(),
  })),
}));

import { useSyncStateStore } from "@/stores/syncStateStore";
import { createMockUIStoreState } from "@/test/mocks";
import { cacheAttachment } from "./cacheManager";
import { startPreCacheManager, stopPreCacheManager } from "./preCacheManager";

const mockInvoke = vi.mocked(invoke);

async function runPreCache() {
  startPreCacheManager();
  await lastRunPromise;
}

describe("preCacheManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    stopPreCacheManager();
    (useSyncStateStore.getState as ReturnType<typeof vi.fn>).mockReturnValue(
      createMockUIStoreState(),
    );
    mockFetchAttachment.mockReset();
  });

  it("skips when offline", async () => {
    (useSyncStateStore.getState as ReturnType<typeof vi.fn>).mockReturnValue(
      createMockUIStoreState({ isOnline: false }),
    );

    await runPreCache();

    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("skips when cache is full", async () => {
    mockInvoke.mockResolvedValueOnce(600 * 1024 * 1024); // cache total size

    await runPreCache();

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("db_attachment_cache_total_size");
  });

  it("fetches and caches uncached attachments", async () => {
    mockInvoke
      .mockResolvedValueOnce(0) // cache total size
      .mockResolvedValueOnce([
        {
          id: "att-1",
          message_id: "msg-1",
          account_id: "acc-1",
          size: 1024,
          gmail_attachment_id: "gmail-att-1",
          imap_part_id: null,
        },
      ]);

    mockFetchAttachment.mockResolvedValueOnce({ data: btoa("hello") });

    await runPreCache();

    expect(mockFetchAttachment).toHaveBeenCalledWith("msg-1", "gmail-att-1");
    expect(cacheAttachment).toHaveBeenCalledWith(
      "att-1",
      expect.any(Uint8Array),
    );
  });

  it("uses imap_part_id when gmail_attachment_id is null", async () => {
    mockInvoke
      .mockResolvedValueOnce(0)
      .mockResolvedValueOnce([
        {
          id: "att-2",
          message_id: "msg-2",
          account_id: "acc-2",
          size: 2048,
          gmail_attachment_id: null,
          imap_part_id: "1.2",
        },
      ]);

    mockFetchAttachment.mockResolvedValueOnce({ data: btoa("data") });

    await runPreCache();

    expect(mockFetchAttachment).toHaveBeenCalledWith("msg-2", "1.2");
  });

  it("skips attachments without any attachment id", async () => {
    mockInvoke
      .mockResolvedValueOnce(0)
      .mockResolvedValueOnce([
        {
          id: "att-3",
          message_id: "msg-3",
          account_id: "acc-3",
          size: 512,
          gmail_attachment_id: null,
          imap_part_id: null,
        },
      ]);

    await runPreCache();

    expect(mockFetchAttachment).not.toHaveBeenCalled();
  });

  it("silently skips on fetch error", async () => {
    mockInvoke
      .mockResolvedValueOnce(0)
      .mockResolvedValueOnce([
        {
          id: "att-4",
          message_id: "msg-4",
          account_id: "acc-4",
          size: 1024,
          gmail_attachment_id: "gmail-att-4",
          imap_part_id: null,
        },
      ]);

    mockFetchAttachment.mockRejectedValueOnce(new Error("network error"));

    await runPreCache();

    expect(cacheAttachment).not.toHaveBeenCalled();
  });

  it("stops when cache limit would be exceeded", async () => {
    const maxBytes = 500 * 1024 * 1024;
    const nearLimit = maxBytes - 100;

    mockInvoke
      .mockResolvedValueOnce(nearLimit) // cache total size
      .mockResolvedValueOnce([
        {
          id: "att-5",
          message_id: "msg-5",
          account_id: "acc-5",
          size: 1024,
          gmail_attachment_id: "gmail-att-5",
          imap_part_id: null,
        },
      ]);

    await runPreCache();

    expect(mockFetchAttachment).not.toHaveBeenCalled();
  });
});
