import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteLocalDraft,
  getLocalDraft,
  getUnsyncedDrafts,
  markDraftSynced,
  upsertLocalDraft,
} from "./localDrafts";

const mockInvoke = vi.mocked(invoke);

describe("localDrafts DB service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("upsertLocalDraft", () => {
    it("inserts or updates a draft", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await upsertLocalDraft({
        id: "draft-1",
        account_id: "acct-1",
        to_addresses: "user@example.com",
        subject: "Test",
        body_html: "<p>Hello</p>",
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_save_local_draft", {
        id: "draft-1",
        accountId: "acct-1",
        toAddresses: "user@example.com",
        ccAddresses: null,
        bccAddresses: null,
        subject: "Test",
        bodyHtml: "<p>Hello</p>",
        replyToMessageId: null,
        threadId: null,
        fromEmail: null,
        signatureId: null,
        remoteDraftId: null,
        attachments: null,
      });
    });

    it("passes null for undefined optional fields", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await upsertLocalDraft({ id: "draft-2", account_id: "acct-1" });

      expect(mockInvoke).toHaveBeenCalledWith("db_save_local_draft", {
        id: "draft-2",
        accountId: "acct-1",
        toAddresses: null,
        ccAddresses: null,
        bccAddresses: null,
        subject: null,
        bodyHtml: null,
        replyToMessageId: null,
        threadId: null,
        fromEmail: null,
        signatureId: null,
        remoteDraftId: null,
        attachments: null,
      });
    });
  });

  describe("getLocalDraft", () => {
    it("returns draft by id", async () => {
      const draft = { id: "draft-1", account_id: "acct-1", subject: "Test" };
      mockInvoke.mockResolvedValueOnce(draft);

      const result = await getLocalDraft("draft-1");

      expect(result).toEqual(draft);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_local_draft", {
        id: "draft-1",
      });
    });

    it("returns null when not found", async () => {
      mockInvoke.mockResolvedValueOnce(null);

      const result = await getLocalDraft("nonexistent");

      expect(result).toBeNull();
    });
  });

  describe("getUnsyncedDrafts", () => {
    it("calls invoke with correct command", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      await getUnsyncedDrafts("acct-1");

      expect(mockInvoke).toHaveBeenCalledWith("db_get_unsynced_drafts", {
        accountId: "acct-1",
      });
    });
  });

  describe("markDraftSynced", () => {
    it("calls invoke with correct params", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await markDraftSynced("draft-1", "remote-123");

      expect(mockInvoke).toHaveBeenCalledWith("db_mark_draft_synced", {
        id: "draft-1",
        remoteDraftId: "remote-123",
      });
    });
  });

  describe("deleteLocalDraft", () => {
    it("calls invoke with correct params", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await deleteLocalDraft("draft-1");

      expect(mockInvoke).toHaveBeenCalledWith("db_delete_local_draft", {
        id: "draft-1",
      });
    });
  });
});
