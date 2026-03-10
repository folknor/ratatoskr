import { vi } from "vitest";
import {
  clearAllFolderSyncStates,
  deleteFolderSyncState,
  type FolderSyncState,
  getAllFolderSyncStates,
  getFolderSyncState,
  upsertFolderSyncState,
} from "./folderSyncState";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("folderSyncState", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getFolderSyncState", () => {
    it("returns null for non-existent folder sync state", async () => {
      mockInvoke.mockResolvedValue(null);

      const result = await getFolderSyncState("acc-1", "INBOX");

      expect(result).toBeNull();
      expect(mockInvoke).toHaveBeenCalledWith("db_get_folder_sync_state", {
        accountId: "acc-1",
        folderPath: "INBOX",
      });
    });

    it("returns existing folder sync state", async () => {
      const state: FolderSyncState = {
        account_id: "acc-1",
        folder_path: "INBOX",
        uidvalidity: 12345,
        last_uid: 100,
        modseq: 999,
        last_sync_at: 1700000000,
      };
      mockInvoke.mockResolvedValue(state);

      const result = await getFolderSyncState("acc-1", "INBOX");

      expect(result).toEqual(state);
    });

    it("passes correct parameters for different folder paths", async () => {
      mockInvoke.mockResolvedValue(null);

      await getFolderSyncState("acc-2", "Sent");

      expect(mockInvoke).toHaveBeenCalledWith("db_get_folder_sync_state", {
        accountId: "acc-2",
        folderPath: "Sent",
      });
    });
  });

  describe("upsertFolderSyncState", () => {
    it("creates new state via invoke", async () => {
      mockInvoke.mockResolvedValue(undefined);

      const state: FolderSyncState = {
        account_id: "acc-1",
        folder_path: "INBOX",
        uidvalidity: 12345,
        last_uid: 100,
        modseq: 999,
        last_sync_at: 1700000000,
      };

      await upsertFolderSyncState(state);

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_folder_sync_state", {
        accountId: "acc-1",
        folderPath: "INBOX",
        uidvalidity: 12345,
        lastUid: 100,
        modseq: 999,
        lastSyncAt: 1700000000,
      });
    });

    it("handles null values for optional fields", async () => {
      mockInvoke.mockResolvedValue(undefined);

      const state: FolderSyncState = {
        account_id: "acc-1",
        folder_path: "Drafts",
        uidvalidity: null,
        last_uid: 0,
        modseq: null,
        last_sync_at: null,
      };

      await upsertFolderSyncState(state);

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_folder_sync_state", {
        accountId: "acc-1",
        folderPath: "Drafts",
        uidvalidity: null,
        lastUid: 0,
        modseq: null,
        lastSyncAt: null,
      });
    });

    it("updates existing state on conflict (upsert)", async () => {
      mockInvoke.mockResolvedValue(undefined);

      const state1: FolderSyncState = {
        account_id: "acc-1",
        folder_path: "INBOX",
        uidvalidity: 12345,
        last_uid: 100,
        modseq: 999,
        last_sync_at: 1700000000,
      };
      await upsertFolderSyncState(state1);

      const state2: FolderSyncState = {
        account_id: "acc-1",
        folder_path: "INBOX",
        uidvalidity: 12345,
        last_uid: 200,
        modseq: 1500,
        last_sync_at: 1700001000,
      };
      await upsertFolderSyncState(state2);

      expect(mockInvoke).toHaveBeenCalledTimes(2);
      expect(mockInvoke).toHaveBeenLastCalledWith(
        "db_upsert_folder_sync_state",
        {
          accountId: "acc-1",
          folderPath: "INBOX",
          uidvalidity: 12345,
          lastUid: 200,
          modseq: 1500,
          lastSyncAt: 1700001000,
        },
      );
    });
  });

  describe("deleteFolderSyncState", () => {
    it("deletes by account_id and folder_path", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await deleteFolderSyncState("acc-1", "INBOX");

      expect(mockInvoke).toHaveBeenCalledWith("db_delete_folder_sync_state", {
        accountId: "acc-1",
        folderPath: "INBOX",
      });
    });

    it("invokes correct command for different folders", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await deleteFolderSyncState("acc-2", "Sent");

      expect(mockInvoke).toHaveBeenCalledWith("db_delete_folder_sync_state", {
        accountId: "acc-2",
        folderPath: "Sent",
      });
    });
  });

  describe("clearAllFolderSyncStates", () => {
    it("clears all states for an account", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await clearAllFolderSyncStates("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_clear_all_folder_sync_states",
        { accountId: "acc-1" },
      );
    });
  });

  describe("getAllFolderSyncStates", () => {
    it("returns all states for an account", async () => {
      const states: FolderSyncState[] = [
        {
          account_id: "acc-1",
          folder_path: "Drafts",
          uidvalidity: 111,
          last_uid: 10,
          modseq: null,
          last_sync_at: 1700000000,
        },
        {
          account_id: "acc-1",
          folder_path: "INBOX",
          uidvalidity: 222,
          last_uid: 50,
          modseq: 500,
          last_sync_at: 1700000000,
        },
        {
          account_id: "acc-1",
          folder_path: "Sent",
          uidvalidity: 333,
          last_uid: 30,
          modseq: null,
          last_sync_at: 1700000000,
        },
      ];
      mockInvoke.mockResolvedValue(states);

      const result = await getAllFolderSyncStates("acc-1");

      expect(result).toEqual(states);
      expect(result).toHaveLength(3);
    });

    it("returns empty array when no states exist", async () => {
      mockInvoke.mockResolvedValue([]);

      const result = await getAllFolderSyncStates("acc-nonexistent");

      expect(result).toEqual([]);
    });

    it("invokes with correct account_id", async () => {
      mockInvoke.mockResolvedValue([]);

      await getAllFolderSyncStates("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith("db_get_all_folder_sync_states", {
        accountId: "acc-1",
      });
    });
  });
});
