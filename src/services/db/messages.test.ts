import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@/core/rustDb", () => ({
  bodyStorePut: vi.fn(),
  bodyStoreGetBatch: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteAllMessagesForAccount,
  updateMessageThreadIds,
} from "./messages";

describe("messages service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("deleteAllMessagesForAccount", () => {
    it("invokes db_delete_all_messages_for_account", async () => {
      await deleteAllMessagesForAccount("acc-1");

      expect(invoke).toHaveBeenCalledWith(
        "db_delete_all_messages_for_account",
        {
          accountId: "acc-1",
        },
      );
    });
  });

  describe("updateMessageThreadIds", () => {
    it("invokes db_update_message_thread_ids", async () => {
      await updateMessageThreadIds(
        "acc-1",
        ["msg-1", "msg-2", "msg-3"],
        "thread-abc",
      );

      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke).toHaveBeenCalledWith("db_update_message_thread_ids", {
        accountId: "acc-1",
        messageIds: ["msg-1", "msg-2", "msg-3"],
        threadId: "thread-abc",
      });
    });

    it("handles empty message list without calling invoke", async () => {
      await updateMessageThreadIds("acc-1", [], "thread-abc");

      expect(invoke).not.toHaveBeenCalled();
    });
  });
});
