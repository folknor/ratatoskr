import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockInvoke } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockInvoke,
}));

import {
  clearProviderClients,
  getActiveProviderName,
  isAiAvailable,
} from "@/services/ai/providerManager";

describe("providerManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getActiveProviderName", () => {
    it("reads the active provider from rust", async () => {
      mockInvoke.mockResolvedValue("openai");

      expect(await getActiveProviderName()).toBe("openai");
      expect(mockInvoke).toHaveBeenCalledWith("ai_get_provider_name");
    });
  });

  describe("isAiAvailable", () => {
    it("returns rust availability results", async () => {
      mockInvoke.mockResolvedValue(true);

      expect(await isAiAvailable()).toBe(true);
      expect(mockInvoke).toHaveBeenCalledWith("ai_is_available");
    });

    it("returns false when the rust command fails", async () => {
      mockInvoke.mockRejectedValue(new Error("boom"));

      expect(await isAiAvailable()).toBe(false);
    });
  });

  describe("clearProviderClients", () => {
    it("is a no-op", () => {
      expect(() => clearProviderClients()).not.toThrow();
    });
  });
});
