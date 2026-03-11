import { beforeEach, describe, expect, it, vi } from "vitest";
import { AiError } from "@/services/ai/errors";

const { mockInvoke } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockInvoke,
}));

import {
  clearProviderClients,
  getActiveProvider,
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

  describe("getActiveProvider", () => {
    it("returns a provider that delegates completions to rust", async () => {
      mockInvoke.mockResolvedValue("result");

      const provider = await getActiveProvider();
      const result = await provider.complete({
        systemPrompt: "system",
        userContent: "user",
      });

      expect(result).toBe("result");
      expect(mockInvoke).toHaveBeenLastCalledWith("ai_complete", {
        request: {
          systemPrompt: "system",
          userContent: "user",
        },
      });
    });

    it("maps typed rust errors on completion", async () => {
      mockInvoke.mockRejectedValue("AUTH_ERROR: Invalid API key");

      const provider = await getActiveProvider();

      await expect(
        provider.complete({
          systemPrompt: "system",
          userContent: "user",
        }),
      ).rejects.toEqual(new AiError("AUTH_ERROR", "Invalid API key"));
    });

    it("delegates connection tests to rust", async () => {
      mockInvoke.mockResolvedValue(true);

      const provider = await getActiveProvider();

      expect(await provider.testConnection()).toBe(true);
      expect(mockInvoke).toHaveBeenCalledWith("ai_test_connection");
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
