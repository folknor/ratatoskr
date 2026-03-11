import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  getAttachmentSenders,
  getAttachmentsForAccount,
  getAttachmentsForMessage,
  upsertAttachment,
} from "@/services/db/attachments";

const mockInvoke = vi.mocked(invoke);

describe("attachments DB service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getAttachmentsForAccount", () => {
    it("calls invoke with correct command and params", async () => {
      const mockData = [
        {
          id: "att-1",
          filename: "test.pdf",
          from_address: "alice@example.com",
          date: 1000,
        },
      ];
      mockInvoke.mockResolvedValueOnce(mockData);

      const result = await getAttachmentsForAccount("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_attachments_for_account",
        {
          accountId: "acc-1",
          limit: 200,
          offset: 0,
        },
      );
      expect(result).toEqual(mockData);
    });

    it("supports custom limit and offset", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      await getAttachmentsForAccount("acc-1", 50, 100);

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_attachments_for_account",
        {
          accountId: "acc-1",
          limit: 50,
          offset: 100,
        },
      );
    });
  });

  describe("getAttachmentSenders", () => {
    it("calls invoke with correct command", async () => {
      const mockSenders = [
        { from_address: "alice@example.com", from_name: "Alice", count: 5 },
      ];
      mockInvoke.mockResolvedValueOnce(mockSenders);

      const result = await getAttachmentSenders("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith("db_get_attachment_senders", {
        accountId: "acc-1",
      });
      expect(result).toEqual(mockSenders);
    });
  });

  describe("upsertAttachment", () => {
    it("calls invoke with correct params", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await upsertAttachment({
        id: "att-1",
        messageId: "msg-1",
        accountId: "acc-1",
        filename: "test.pdf",
        mimeType: "application/pdf",
        size: 1024,
        attachmentId: "gid-1",
        contentId: null,
        isInline: false,
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_attachment", {
        id: "att-1",
        messageId: "msg-1",
        accountId: "acc-1",
        filename: "test.pdf",
        mimeType: "application/pdf",
        size: 1024,
        attachmentId: "gid-1",
        contentId: null,
        isInline: false,
      });
    });
  });

  describe("getAttachmentsForMessage", () => {
    it("calls invoke for a specific message", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      await getAttachmentsForMessage("acc-1", "msg-1");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_attachments_for_message",
        {
          accountId: "acc-1",
          messageId: "msg-1",
        },
      );
    });
  });
});
