import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { getAllowlistedSenders } from "./imageAllowlist";

describe("imageAllowlist service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getAllowlistedSenders", () => {
    it("returns empty set for empty senders array", async () => {
      const result = await getAllowlistedSenders("acc-1", []);
      expect(result.size).toBe(0);
      expect(invoke).not.toHaveBeenCalled();
    });

    it("returns set of allowlisted senders from batch query", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([
        "alice@example.com",
        "bob@example.com",
      ]);

      const result = await getAllowlistedSenders("acc-1", [
        "alice@example.com",
        "bob@example.com",
        "carol@example.com",
      ]);

      expect(result.size).toBe(2);
      expect(result.has("alice@example.com")).toBe(true);
      expect(result.has("bob@example.com")).toBe(true);
      expect(result.has("carol@example.com")).toBe(false);
    });

    it("calls Rust command with normalized addresses", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([]);

      await getAllowlistedSenders("acc-1", ["a@example.com", "b@example.com"]);

      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke).toHaveBeenCalledWith("db_get_allowlisted_senders", {
        accountId: "acc-1",
        senderAddresses: ["a@example.com", "b@example.com"],
      });
    });
  });
});
