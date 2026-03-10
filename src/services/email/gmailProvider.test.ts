import { vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { GmailApiProvider } from "./gmailProvider";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("GmailApiProvider", () => {
  let provider: GmailApiProvider;

  beforeEach(() => {
    vi.clearAllMocks();
    provider = new GmailApiProvider("account-1");
  });

  it("has correct accountId and type", () => {
    expect(provider.accountId).toBe("account-1");
    expect(provider.type).toBe("gmail_api");
  });

  describe("listFolders", () => {
    it("maps Gmail labels to EmailFolder format", async () => {
      mockInvoke.mockResolvedValue([
        {
          id: "INBOX",
          name: "INBOX",
          labelType: "system",
          messagesTotal: 100,
          messagesUnread: 5,
        },
        {
          id: "SENT",
          name: "SENT",
          labelType: "system",
          messagesTotal: 50,
          messagesUnread: 0,
        },
        {
          id: "Label_1",
          name: "My Label",
          labelType: "user",
          messagesTotal: 10,
          messagesUnread: 2,
        },
      ]);

      const folders = await provider.listFolders();

      expect(mockInvoke).toHaveBeenCalledWith("gmail_list_labels", {
        accountId: "account-1",
      });
      expect(folders).toHaveLength(3);
      expect(folders[0]).toEqual({
        id: "INBOX",
        name: "INBOX",
        path: "INBOX",
        type: "system",
        specialUse: null,
        delimiter: "/",
        messageCount: 100,
        unreadCount: 5,
      });
      expect(folders[1]).toEqual({
        id: "SENT",
        name: "SENT",
        path: "SENT",
        type: "system",
        specialUse: "\\Sent",
        delimiter: "/",
        messageCount: 50,
        unreadCount: 0,
      });
      expect(folders[2]).toEqual({
        id: "Label_1",
        name: "My Label",
        path: "My Label",
        type: "user",
        specialUse: null,
        delimiter: "/",
        messageCount: 10,
        unreadCount: 2,
      });
    });

    it("maps special-use flags for system labels", async () => {
      mockInvoke.mockResolvedValue([
        { id: "TRASH", name: "TRASH", labelType: "system", messagesTotal: null, messagesUnread: null },
        { id: "DRAFT", name: "DRAFT", labelType: "system", messagesTotal: null, messagesUnread: null },
        { id: "SPAM", name: "SPAM", labelType: "system", messagesTotal: null, messagesUnread: null },
      ]);

      const folders = await provider.listFolders();

      expect(folders[0]?.specialUse).toBe("\\Trash");
      expect(folders[1]?.specialUse).toBe("\\Drafts");
      expect(folders[2]?.specialUse).toBe("\\Junk");
    });
  });

  describe("createFolder", () => {
    it("creates a label and returns EmailFolder", async () => {
      mockInvoke.mockResolvedValue({
        id: "Label_new",
        name: "New Folder",
        labelType: "user",
      });

      const folder = await provider.createFolder("New Folder");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_create_label", {
        accountId: "account-1",
        name: "New Folder",
      });
      expect(folder.id).toBe("Label_new");
      expect(folder.name).toBe("New Folder");
      expect(folder.type).toBe("user");
    });

    it("prepends parent path when provided", async () => {
      mockInvoke.mockResolvedValue({
        id: "Label_nested",
        name: "Parent/Child",
        labelType: "user",
      });

      await provider.createFolder("Child", "Parent");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_create_label", {
        accountId: "account-1",
        name: "Parent/Child",
      });
    });
  });

  describe("archive", () => {
    it("calls modifyThread removing INBOX label", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.archive("thread-1", ["msg-1"]);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: [],
        removeLabels: ["INBOX"],
      });
    });
  });

  describe("trash", () => {
    it("calls modifyThread adding TRASH and removing INBOX", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.trash("thread-1", ["msg-1"]);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: ["TRASH"],
        removeLabels: ["INBOX"],
      });
    });
  });

  describe("permanentDelete", () => {
    it("calls deleteThread", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await provider.permanentDelete("thread-1", ["msg-1"]);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_delete_thread", {
        accountId: "account-1",
        threadId: "thread-1",
      });
    });
  });

  describe("markRead", () => {
    it("removes UNREAD label when marking as read", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.markRead("thread-1", ["msg-1"], true);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: [],
        removeLabels: ["UNREAD"],
      });
    });

    it("adds UNREAD label when marking as unread", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.markRead("thread-1", ["msg-1"], false);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: ["UNREAD"],
        removeLabels: [],
      });
    });
  });

  describe("star", () => {
    it("adds STARRED label when starring", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.star("thread-1", ["msg-1"], true);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: ["STARRED"],
        removeLabels: [],
      });
    });

    it("removes STARRED label when unstarring", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.star("thread-1", ["msg-1"], false);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: [],
        removeLabels: ["STARRED"],
      });
    });
  });

  describe("spam", () => {
    it("adds SPAM and removes INBOX when marking as spam", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.spam("thread-1", ["msg-1"], true);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: ["SPAM"],
        removeLabels: ["INBOX"],
      });
    });

    it("adds INBOX and removes SPAM when marking as not spam", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.spam("thread-1", ["msg-1"], false);

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: ["INBOX"],
        removeLabels: ["SPAM"],
      });
    });
  });

  describe("sendMessage", () => {
    it("delegates to gmail_send_email and returns id", async () => {
      mockInvoke.mockResolvedValue({ id: "sent-msg-1" });

      const result = await provider.sendMessage("base64data", "thread-1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_send_email", {
        accountId: "account-1",
        raw: "base64data",
        threadId: "thread-1",
      });
      expect(result).toEqual({ id: "sent-msg-1" });
    });
  });

  describe("createDraft", () => {
    it("delegates to gmail_create_draft and returns draftId", async () => {
      mockInvoke.mockResolvedValue({
        id: "draft-1",
        message: { id: "msg-1", threadId: "thread-1" },
      });

      const result = await provider.createDraft("base64data", "thread-1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_create_draft", {
        accountId: "account-1",
        raw: "base64data",
        threadId: "thread-1",
      });
      expect(result).toEqual({ draftId: "draft-1" });
    });
  });

  describe("updateDraft", () => {
    it("delegates to gmail_update_draft and returns draftId", async () => {
      mockInvoke.mockResolvedValue({
        id: "draft-1",
        message: { id: "msg-1", threadId: "thread-1" },
      });

      const result = await provider.updateDraft(
        "draft-1",
        "base64data",
        "thread-1",
      );

      expect(mockInvoke).toHaveBeenCalledWith("gmail_update_draft", {
        accountId: "account-1",
        draftId: "draft-1",
        raw: "base64data",
        threadId: "thread-1",
      });
      expect(result).toEqual({ draftId: "draft-1" });
    });
  });

  describe("deleteDraft", () => {
    it("delegates to gmail_delete_draft", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await provider.deleteDraft("draft-1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_delete_draft", {
        accountId: "account-1",
        draftId: "draft-1",
      });
    });
  });

  describe("testConnection", () => {
    it("returns success when getProfile succeeds", async () => {
      mockInvoke.mockResolvedValue({
        emailAddress: "user@gmail.com",
        messagesTotal: 1000,
        threadsTotal: 500,
        historyId: "12345",
      });

      const result = await provider.testConnection();

      expect(result).toEqual({
        success: true,
        message: "Connected as user@gmail.com",
      });
    });

    it("returns failure when getProfile throws", async () => {
      mockInvoke.mockRejectedValue(new Error("Token expired"));

      const result = await provider.testConnection();

      expect(result).toEqual({
        success: false,
        message: "Token expired",
      });
    });
  });

  describe("getProfile", () => {
    it("returns email from Gmail profile", async () => {
      mockInvoke.mockResolvedValue({
        emailAddress: "user@gmail.com",
        messagesTotal: 1000,
        threadsTotal: 500,
        historyId: "12345",
      });

      const result = await provider.getProfile();

      expect(result).toEqual({ email: "user@gmail.com" });
    });
  });

  describe("fetchRawMessage", () => {
    it("fetches raw format and decodes base64url to string", async () => {
      const rawContent = "From: test@example.com\r\nSubject: Hi\r\n\r\nHello";
      const base64url = btoa(rawContent)
        .replace(/\+/g, "-")
        .replace(/\//g, "_")
        .replace(/=+$/, "");
      mockInvoke.mockResolvedValue({
        id: "msg-1",
        threadId: "thread-1",
        raw: base64url,
      });

      const result = await provider.fetchRawMessage("msg-1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_get_message", {
        accountId: "account-1",
        messageId: "msg-1",
        format: "raw",
      });
      expect(result).toBe(rawContent);
    });
  });

  describe("fetchAttachment", () => {
    it("delegates to gmail_fetch_attachment", async () => {
      mockInvoke.mockResolvedValue({
        attachmentId: "att-1",
        size: 1024,
        data: "base64data",
      });

      const result = await provider.fetchAttachment("msg-1", "att-1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_fetch_attachment", {
        accountId: "account-1",
        messageId: "msg-1",
        attachmentId: "att-1",
      });
      expect(result).toEqual({ data: "base64data", size: 1024 });
    });
  });

  describe("addLabel / removeLabel", () => {
    it("addLabel calls modifyThread with add", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.addLabel("thread-1", "Label_1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: ["Label_1"],
        removeLabels: [],
      });
    });

    it("removeLabel calls modifyThread with remove", async () => {
      mockInvoke.mockResolvedValue({});

      await provider.removeLabel("thread-1", "Label_1");

      expect(mockInvoke).toHaveBeenCalledWith("gmail_modify_thread", {
        accountId: "account-1",
        threadId: "thread-1",
        addLabels: [],
        removeLabels: ["Label_1"],
      });
    });
  });
});
