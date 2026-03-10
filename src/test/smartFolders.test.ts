import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteSmartFolder,
  getSmartFolderById,
  getSmartFolders,
  insertSmartFolder,
  updateSmartFolder,
  updateSmartFolderSortOrder,
} from "./smartFolders";

describe("smartFolders service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getSmartFolders", () => {
    it("returns global folders when no accountId", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([]);

      await getSmartFolders();

      expect(invoke).toHaveBeenCalledWith("db_get_smart_folders", {
        accountId: null,
      });
    });

    it("returns global + account folders when accountId provided", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([]);

      await getSmartFolders("acc-1");

      expect(invoke).toHaveBeenCalledWith("db_get_smart_folders", {
        accountId: "acc-1",
      });
    });
  });

  describe("getSmartFolderById", () => {
    it("returns the folder when found", async () => {
      const mockFolder = {
        id: "sf-1",
        account_id: null,
        name: "Unread",
        query: "is:unread",
        icon: "MailOpen",
        color: null,
        sort_order: 0,
        is_default: true,
        created_at: 1234567890,
      };
      vi.mocked(invoke).mockResolvedValueOnce(mockFolder);

      const result = await getSmartFolderById("sf-1");

      expect(result).toEqual(mockFolder);
      expect(invoke).toHaveBeenCalledWith("db_get_smart_folder_by_id", {
        id: "sf-1",
      });
    });

    it("returns null when not found", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(null);

      const result = await getSmartFolderById("nonexistent");

      expect(result).toBeNull();
    });
  });

  describe("insertSmartFolder", () => {
    it("inserts with all fields", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      const id = await insertSmartFolder({
        name: "Test Folder",
        query: "is:unread",
        accountId: "acc-1",
        icon: "Star",
        color: "#ff0000",
      });

      expect(id).toBeTruthy();
      expect(invoke).toHaveBeenCalledWith("db_insert_smart_folder", {
        id: expect.any(String),
        name: "Test Folder",
        query: "is:unread",
        accountId: "acc-1",
        icon: "Star",
        color: "#ff0000",
      });
    });

    it("inserts with defaults for optional fields", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await insertSmartFolder({
        name: "Test",
        query: "from:boss",
      });

      expect(invoke).toHaveBeenCalledWith("db_insert_smart_folder", {
        id: expect.any(String),
        name: "Test",
        query: "from:boss",
        accountId: null,
        icon: null,
        color: null,
      });
    });
  });

  describe("updateSmartFolder", () => {
    it("passes updates via invoke", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await updateSmartFolder("sf-1", { name: "New Name" });

      expect(invoke).toHaveBeenCalledWith("db_update_smart_folder", {
        id: "sf-1",
        name: "New Name",
      });
    });

    it("sends only provided fields", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await updateSmartFolder("sf-1", {});

      expect(invoke).toHaveBeenCalledWith("db_update_smart_folder", {
        id: "sf-1",
      });
    });
  });

  describe("deleteSmartFolder", () => {
    it("deletes by id", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await deleteSmartFolder("sf-1");

      expect(invoke).toHaveBeenCalledWith("db_delete_smart_folder", {
        id: "sf-1",
      });
    });
  });

  describe("updateSmartFolderSortOrder", () => {
    it("updates sort_order for each item", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await updateSmartFolderSortOrder([
        { id: "sf-1", sortOrder: 2 },
        { id: "sf-2", sortOrder: 0 },
      ]);

      expect(invoke).toHaveBeenCalledWith("db_update_smart_folder_sort_order", {
        orders: [
          { id: "sf-1", sort_order: 2 },
          { id: "sf-2", sort_order: 0 },
        ],
      });
    });
  });
});
