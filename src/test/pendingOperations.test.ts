import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import {
  clearFailedOperations,
  compactQueue,
  deleteOperation,
  enqueuePendingOperation,
  getFailedOpsCount,
  getPendingOperations,
  getPendingOpsCount,
  getPendingOpsForResource,
  incrementRetry,
  retryFailedOperations,
  updateOperationStatus,
} from "@/services/db/pendingOperations";

const mockInvoke = vi.mocked(invoke);

describe("pendingOperations DB service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("enqueuePendingOperation", () => {
    it("invokes the Rust command with UUID", async () => {
      const id = await enqueuePendingOperation(
        "acct-1",
        "archive",
        "thread-1",
        { threadId: "thread-1" },
      );
      expect(id).toBeTruthy();
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_enqueue", {
        id: expect.any(String),
        accountId: "acct-1",
        operationType: "archive",
        resourceId: "thread-1",
        paramsJson: JSON.stringify({ threadId: "thread-1" }),
      });
    });
  });

  describe("getPendingOperations", () => {
    it("fetches pending ops for a specific account", async () => {
      mockInvoke.mockResolvedValueOnce([]);
      await getPendingOperations("acct-1");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_get", {
        accountId: "acct-1",
        limit: 50,
      });
    });

    it("fetches all pending ops when no account specified", async () => {
      mockInvoke.mockResolvedValueOnce([]);
      await getPendingOperations();
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_get", {
        accountId: null,
        limit: 50,
      });
    });

    it("respects custom limit", async () => {
      mockInvoke.mockResolvedValueOnce([]);
      await getPendingOperations(undefined, 10);
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_get", {
        accountId: null,
        limit: 10,
      });
    });
  });

  describe("updateOperationStatus", () => {
    it("updates the status and error message", async () => {
      await updateOperationStatus("op-1", "failed", "Network timeout");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_update_status", {
        id: "op-1",
        status: "failed",
        errorMessage: "Network timeout",
      });
    });

    it("passes null error_message when not provided", async () => {
      await updateOperationStatus("op-1", "pending");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_update_status", {
        id: "op-1",
        status: "pending",
        errorMessage: null,
      });
    });
  });

  describe("deleteOperation", () => {
    it("deletes by id", async () => {
      await deleteOperation("op-1");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_delete", {
        id: "op-1",
      });
    });
  });

  describe("incrementRetry", () => {
    it("invokes the Rust command", async () => {
      await incrementRetry("op-1");
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_pending_ops_increment_retry",
        { id: "op-1" },
      );
    });
  });

  describe("getPendingOpsCount", () => {
    it("returns count for specific account", async () => {
      mockInvoke.mockResolvedValueOnce(5);
      const count = await getPendingOpsCount("acct-1");
      expect(count).toBe(5);
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_count", {
        accountId: "acct-1",
      });
    });

    it("returns global count", async () => {
      mockInvoke.mockResolvedValueOnce(12);
      const count = await getPendingOpsCount();
      expect(count).toBe(12);
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_count", {
        accountId: null,
      });
    });
  });

  describe("getFailedOpsCount", () => {
    it("returns count of failed operations", async () => {
      mockInvoke.mockResolvedValueOnce(3);
      const count = await getFailedOpsCount();
      expect(count).toBe(3);
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_failed_count", {
        accountId: null,
      });
    });
  });

  describe("getPendingOpsForResource", () => {
    it("queries by account and resource", async () => {
      mockInvoke.mockResolvedValueOnce([]);
      await getPendingOpsForResource("acct-1", "thread-1");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_for_resource", {
        accountId: "acct-1",
        resourceId: "thread-1",
      });
    });
  });

  describe("compactQueue", () => {
    it("invokes the Rust compact command", async () => {
      mockInvoke.mockResolvedValueOnce(2);
      const removed = await compactQueue();
      expect(removed).toBe(2);
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_compact", {
        accountId: null,
      });
    });

    it("passes account_id when provided", async () => {
      mockInvoke.mockResolvedValueOnce(0);
      await compactQueue("acct-1");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_compact", {
        accountId: "acct-1",
      });
    });
  });

  describe("clearFailedOperations", () => {
    it("clears all failed ops", async () => {
      await clearFailedOperations();
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_clear_failed", {
        accountId: null,
      });
    });

    it("clears failed ops for specific account", async () => {
      await clearFailedOperations("acct-1");
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_clear_failed", {
        accountId: "acct-1",
      });
    });
  });

  describe("retryFailedOperations", () => {
    it("resets failed ops to pending", async () => {
      await retryFailedOperations();
      expect(mockInvoke).toHaveBeenCalledWith("db_pending_ops_retry_failed", {
        accountId: null,
      });
    });
  });
});
