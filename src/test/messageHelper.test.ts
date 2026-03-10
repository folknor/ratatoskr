import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  findSpecialFolder,
  getImapUidsForMessages,
  groupMessagesByFolder,
  type ImapMessageInfo,
  securityToConfigType,
  updateMessageImapFolder,
} from "./messageHelper";

const mockInvoke = vi.mocked(invoke);

describe("messageHelper", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("groupMessagesByFolder", () => {
    it("groups messages by their folder", () => {
      const messages = new Map<string, ImapMessageInfo>([
        ["msg1", { uid: 100, folder: "INBOX" }],
        ["msg2", { uid: 200, folder: "INBOX" }],
        ["msg3", { uid: 300, folder: "Sent" }],
        ["msg4", { uid: 400, folder: "Drafts" }],
      ]);

      const grouped = groupMessagesByFolder(messages);

      expect(grouped.size).toBe(3);
      expect(grouped.get("INBOX")).toEqual([100, 200]);
      expect(grouped.get("Sent")).toEqual([300]);
      expect(grouped.get("Drafts")).toEqual([400]);
    });

    it("returns empty map for empty input", () => {
      const messages = new Map<string, ImapMessageInfo>();
      const grouped = groupMessagesByFolder(messages);
      expect(grouped.size).toBe(0);
    });

    it("handles single message", () => {
      const messages = new Map<string, ImapMessageInfo>([
        ["msg1", { uid: 42, folder: "Archive" }],
      ]);

      const grouped = groupMessagesByFolder(messages);
      expect(grouped.size).toBe(1);
      expect(grouped.get("Archive")).toEqual([42]);
    });
  });

  describe("securityToConfigType", () => {
    it("maps 'ssl' to 'tls'", () => {
      expect(securityToConfigType("ssl")).toBe("tls");
    });

    it("maps 'starttls' to 'starttls'", () => {
      expect(securityToConfigType("starttls")).toBe("starttls");
    });

    it("maps 'none' to 'none'", () => {
      expect(securityToConfigType("none")).toBe("none");
    });

    it("defaults to 'tls' for unknown values", () => {
      expect(securityToConfigType("unknown")).toBe("tls");
      expect(securityToConfigType("")).toBe("tls");
    });
  });

  describe("getImapUidsForMessages", () => {
    it("returns empty map for empty input", async () => {
      const result = await getImapUidsForMessages("acc1", []);
      expect(result.size).toBe(0);
    });
  });

  describe("findSpecialFolder", () => {
    it("returns null when no matching folder exists", async () => {
      mockInvoke.mockResolvedValueOnce(null);

      const result = await findSpecialFolder("acc1", "\\Trash");
      expect(result).toBeNull();
      expect(mockInvoke).toHaveBeenCalledWith("db_find_special_folder", {
        accountId: "acc1",
        specialUse: "\\Trash",
        fallbackLabelId: "TRASH",
      });
    });

    it("returns folder path from invoke result", async () => {
      mockInvoke.mockResolvedValueOnce("INBOX.Trash");

      const result = await findSpecialFolder("acc1", "\\Trash");
      expect(result).toBe("INBOX.Trash");
    });
  });

  describe("updateMessageImapFolder", () => {
    it("does nothing for empty message list", async () => {
      await updateMessageImapFolder("acc1", [], "INBOX");
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    it("updates folder for given messages", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await updateMessageImapFolder("acc1", ["msg1", "msg2"], "Trash");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_message_imap_folder", {
        accountId: "acc1",
        messageIds: ["msg1", "msg2"],
        newFolder: "Trash",
      });
    });
  });
});
