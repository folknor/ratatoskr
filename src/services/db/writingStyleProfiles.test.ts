import { vi } from "vitest";
import {
  deleteWritingStyleProfile,
  getWritingStyleProfile,
  upsertWritingStyleProfile,
} from "./writingStyleProfiles";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("writingStyleProfiles", () => {
  describe("getWritingStyleProfile", () => {
    it("returns profile when found", async () => {
      const profile = {
        id: "p1",
        account_id: "acc1",
        profile_text: "Formal tone",
        sample_count: 10,
        created_at: 1000,
        updated_at: 1000,
      };
      mockInvoke.mockResolvedValue(profile);

      const result = await getWritingStyleProfile("acc1");
      expect(result).toEqual(profile);
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_writing_style_profile",
        { accountId: "acc1" },
      );
    });

    it("returns null when not found", async () => {
      mockInvoke.mockResolvedValue(null);
      const result = await getWritingStyleProfile("acc1");
      expect(result).toBeNull();
    });
  });

  describe("upsertWritingStyleProfile", () => {
    it("inserts or updates a profile", async () => {
      mockInvoke.mockResolvedValue(undefined);
      await upsertWritingStyleProfile("acc1", "Casual tone", 15);
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_upsert_writing_style_profile",
        {
          accountId: "acc1",
          profileText: "Casual tone",
          sampleCount: 15,
        },
      );
    });
  });

  describe("deleteWritingStyleProfile", () => {
    it("deletes profile for account", async () => {
      mockInvoke.mockResolvedValue(undefined);
      await deleteWritingStyleProfile("acc1");
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_delete_writing_style_profile",
        { accountId: "acc1" },
      );
    });
  });
});
