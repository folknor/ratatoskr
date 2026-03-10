import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { getBundleSummaries } from "./bundleRules";

const mockInvoke = vi.mocked(invoke);

describe("bundleRules service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getBundleSummaries", () => {
    it("returns empty map for empty categories", async () => {
      const result = await getBundleSummaries("acc-1", []);
      expect(result.size).toBe(0);
      expect(mockInvoke).not.toHaveBeenCalled();
    });

    it("fetches summaries for multiple categories via single invoke", async () => {
      mockInvoke.mockResolvedValueOnce([
        {
          category: "Promotions",
          count: 5,
          latestSubject: "Big Sale",
          latestSender: "Store",
        },
        {
          category: "Social",
          count: 3,
          latestSubject: "New follower",
          latestSender: "App",
        },
      ]);

      const result = await getBundleSummaries("acc-1", [
        "Promotions",
        "Social",
      ]);

      expect(result.size).toBe(2);
      expect(result.get("Promotions")).toEqual({
        count: 5,
        latestSubject: "Big Sale",
        latestSender: "Store",
      });
      expect(result.get("Social")).toEqual({
        count: 3,
        latestSubject: "New follower",
        latestSender: "App",
      });
      expect(mockInvoke).toHaveBeenCalledOnce();
      expect(mockInvoke).toHaveBeenCalledWith("db_get_bundle_summaries", {
        accountId: "acc-1",
        categories: ["Promotions", "Social"],
      });
    });

    it("returns zero counts for categories with no threads", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      const result = await getBundleSummaries("acc-1", ["Empty"]);

      expect(result.get("Empty")).toEqual({
        count: 0,
        latestSubject: null,
        latestSender: null,
      });
    });
  });
});
