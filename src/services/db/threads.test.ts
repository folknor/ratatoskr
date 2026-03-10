import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteAllThreadsForAccount,
  getMutedThreadIds,
  muteThread,
  unmuteThread,
} from "./threads";

describe("threads service - deleteAllThreadsForAccount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("invokes db_delete_all_threads_for_account", async () => {
    await deleteAllThreadsForAccount("acc-1");

    expect(invoke).toHaveBeenCalledWith("db_delete_all_threads_for_account", {
      accountId: "acc-1",
    });
  });
});

describe("threads service - mute", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("muteThread", () => {
    it("invokes db_set_thread_muted with isMuted true", async () => {
      await muteThread("acc-1", "thread-1");

      expect(invoke).toHaveBeenCalledWith("db_set_thread_muted", {
        accountId: "acc-1",
        threadId: "thread-1",
        isMuted: true,
      });
    });
  });

  describe("unmuteThread", () => {
    it("invokes db_set_thread_muted with isMuted false", async () => {
      await unmuteThread("acc-1", "thread-1");

      expect(invoke).toHaveBeenCalledWith("db_set_thread_muted", {
        accountId: "acc-1",
        threadId: "thread-1",
        isMuted: false,
      });
    });
  });

  describe("getMutedThreadIds", () => {
    it("returns a Set of muted thread IDs", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(["thread-1", "thread-3"]);

      const result = await getMutedThreadIds("acc-1");

      expect(invoke).toHaveBeenCalledWith("db_get_muted_thread_ids", {
        accountId: "acc-1",
      });
      expect(result).toBeInstanceOf(Set);
      expect(result.size).toBe(2);
      expect(result.has("thread-1")).toBe(true);
      expect(result.has("thread-3")).toBe(true);
    });

    it("returns an empty Set when no threads are muted", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([]);

      const result = await getMutedThreadIds("acc-1");

      expect(result.size).toBe(0);
    });
  });
});
