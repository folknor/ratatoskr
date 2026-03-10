import { beforeEach, describe, expect, it, vi } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

import {
  deleteContact,
  getAllContacts,
  getAttachmentsFromContact,
  getContactsFromSameDomain,
  getLatestAuthResult,
  updateContact,
  updateContactNotes,
} from "./contacts";

describe("contacts service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getAllContacts", () => {
    it("calls invoke with correct command and default params", async () => {
      mockInvoke.mockResolvedValue([]);

      await getAllContacts();

      expect(mockInvoke).toHaveBeenCalledWith("db_get_all_contacts", {
        limit: 500,
        offset: 0,
      });
    });

    it("passes limit and offset params", async () => {
      mockInvoke.mockResolvedValue([]);

      await getAllContacts(100, 50);

      expect(mockInvoke).toHaveBeenCalledWith("db_get_all_contacts", {
        limit: 100,
        offset: 50,
      });
    });
  });

  describe("updateContact", () => {
    it("calls invoke with correct params", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await updateContact("contact-123", "John Doe");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_contact", {
        id: "contact-123",
        displayName: "John Doe",
      });
    });
  });

  describe("deleteContact", () => {
    it("calls invoke with correct id", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await deleteContact("contact-456");

      expect(mockInvoke).toHaveBeenCalledWith("db_delete_contact", {
        id: "contact-456",
      });
    });
  });

  describe("updateContactNotes", () => {
    it("calls invoke with normalized email", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await updateContactNotes("John@Example.COM", "Great client");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_contact_notes", {
        email: "john@example.com",
        notes: "Great client",
      });
    });

    it("stores null for empty notes", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await updateContactNotes("user@test.com", "");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_contact_notes", {
        email: "user@test.com",
        notes: null,
      });
    });
  });

  describe("getAttachmentsFromContact", () => {
    it("queries with default limit", async () => {
      mockInvoke.mockResolvedValue([]);

      await getAttachmentsFromContact("sender@test.com");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_attachments_from_contact",
        {
          email: "sender@test.com",
          limit: 5,
        },
      );
    });

    it("passes custom limit", async () => {
      mockInvoke.mockResolvedValue([]);

      await getAttachmentsFromContact("sender@test.com", 10);

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_attachments_from_contact",
        {
          email: "sender@test.com",
          limit: 10,
        },
      );
    });
  });

  describe("getContactsFromSameDomain", () => {
    it("queries contacts with same domain", async () => {
      mockInvoke.mockResolvedValue([]);

      await getContactsFromSameDomain("alice@company.com");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_contacts_from_same_domain",
        {
          email: "alice@company.com",
          limit: 5,
        },
      );
    });

    it("returns empty array for public domains", async () => {
      const result = await getContactsFromSameDomain("user@gmail.com");

      expect(result).toEqual([]);
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    it("returns empty array for email without @", async () => {
      const result = await getContactsFromSameDomain("invalid-email");

      expect(result).toEqual([]);
      expect(mockInvoke).not.toHaveBeenCalled();
    });
  });

  describe("getLatestAuthResult", () => {
    it("queries most recent auth_results", async () => {
      mockInvoke.mockResolvedValue('{"aggregate":"pass"}');

      const result = await getLatestAuthResult("sender@test.com");

      expect(result).toBe('{"aggregate":"pass"}');
      expect(mockInvoke).toHaveBeenCalledWith("db_get_latest_auth_result", {
        email: "sender@test.com",
      });
    });

    it("returns null when no results", async () => {
      mockInvoke.mockResolvedValue(null);

      const result = await getLatestAuthResult("unknown@test.com");

      expect(result).toBeNull();
    });
  });
});
