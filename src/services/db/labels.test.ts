import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { updateLabelSortOrder } from "./labels";

const mockInvoke = vi.mocked(invoke);

describe("labels service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("updateLabelSortOrder", () => {
    it("calls invoke with correct params", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      const orders = [
        { id: "label-1", sortOrder: 0 },
        { id: "label-2", sortOrder: 1 },
        { id: "label-3", sortOrder: 2 },
      ];

      await updateLabelSortOrder("acc-1", orders);

      expect(mockInvoke).toHaveBeenCalledWith("db_update_label_sort_order", {
        accountId: "acc-1",
        labelOrders: orders,
      });
    });

    it("handles empty array", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await updateLabelSortOrder("acc-1", []);

      expect(mockInvoke).toHaveBeenCalledWith("db_update_label_sort_order", {
        accountId: "acc-1",
        labelOrders: [],
      });
    });
  });
});
