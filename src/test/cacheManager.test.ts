import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMockTauriFs, createMockTauriPath } from "@/test/mocks";

const tauriFs = createMockTauriFs();
const tauriPath = createMockTauriPath();

vi.mock("@tauri-apps/plugin-fs", () => tauriFs.mock);
vi.mock("@tauri-apps/api/path", () => tauriPath);

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@/services/db/settings", () => ({
  getSetting: vi.fn(() => Promise.resolve("500")),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  cacheAttachment,
  clearAllCache,
  evictOldestCached,
  getCacheSize,
  loadCachedAttachment,
} from "./cacheManager";

const mockInvoke = vi.mocked(invoke);

describe("cacheManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriFs.mock.mkdir.mockResolvedValue(undefined);
    tauriFs.mock.writeFile.mockResolvedValue(undefined);
    tauriFs.mock.readFile.mockResolvedValue(new Uint8Array([1, 2, 3]));
    tauriFs.mock.remove.mockResolvedValue(undefined);
    mockInvoke.mockResolvedValue(undefined);
  });

  describe("cacheAttachment", () => {
    it("creates cache dir with baseDir option", async () => {
      const data = new Uint8Array([10, 20, 30]);
      await cacheAttachment("att-1", data);

      expect(tauriFs.mock.mkdir).toHaveBeenCalledWith("attachment_cache", {
        baseDir: 26,
        recursive: true,
      });
    });

    it("writes file with baseDir option", async () => {
      const data = new Uint8Array([10, 20, 30]);
      await cacheAttachment("att-1", data);

      expect(tauriFs.mock.writeFile).toHaveBeenCalledWith(
        expect.stringContaining("attachment_cache/"),
        data,
        { baseDir: 26 },
      );
    });

    it("returns relative path", async () => {
      const result = await cacheAttachment("att-1", new Uint8Array([1]));
      expect(result).toMatch(/^attachment_cache\//);
    });

    it("updates DB via invoke", async () => {
      const data = new Uint8Array([10, 20]);
      await cacheAttachment("att-1", data);

      expect(mockInvoke).toHaveBeenCalledWith("db_update_attachment_cached", {
        attachmentId: "att-1",
        localPath: expect.stringContaining("attachment_cache/"),
        cacheSize: 2,
      });
    });
  });

  describe("loadCachedAttachment", () => {
    it("reads file with baseDir option", async () => {
      const result = await loadCachedAttachment("attachment_cache/abc");

      expect(tauriFs.mock.readFile).toHaveBeenCalledWith(
        "attachment_cache/abc",
        { baseDir: 26 },
      );
      expect(result).toEqual(new Uint8Array([1, 2, 3]));
    });

    it("returns null on read error", async () => {
      tauriFs.mock.readFile.mockRejectedValueOnce(new Error("not found"));
      const result = await loadCachedAttachment("attachment_cache/missing");
      expect(result).toBeNull();
    });
  });

  describe("getCacheSize", () => {
    it("returns total cache size from invoke", async () => {
      mockInvoke.mockResolvedValueOnce(1024);
      const size = await getCacheSize();
      expect(size).toBe(1024);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_attachment_cache_size");
    });

    it("returns 0 when no cached attachments", async () => {
      mockInvoke.mockResolvedValueOnce(0);
      const size = await getCacheSize();
      expect(size).toBe(0);
    });
  });

  describe("evictOldestCached", () => {
    it("removes files with baseDir option when over limit", async () => {
      const maxBytes = 500 * 1024 * 1024;
      // First invoke: getCacheSize
      mockInvoke
        .mockResolvedValueOnce(maxBytes + 1000)
        // Second invoke: get oldest cached attachments
        .mockResolvedValueOnce([
          {
            id: "att-old",
            local_path: "attachment_cache/old",
            cache_size: 2000,
          },
        ])
        // Third invoke: clear cache entry
        .mockResolvedValueOnce(undefined);

      await evictOldestCached();

      expect(tauriFs.mock.remove).toHaveBeenCalledWith("attachment_cache/old", {
        baseDir: 26,
      });
      expect(mockInvoke).toHaveBeenCalledWith("db_clear_attachment_cache_entry", {
        attachmentId: "att-old",
      });
    });

    it("does nothing when under limit", async () => {
      mockInvoke.mockResolvedValueOnce(100);

      await evictOldestCached();

      expect(tauriFs.mock.remove).not.toHaveBeenCalled();
    });
  });

  describe("clearAllCache", () => {
    it("removes cache dir with baseDir option", async () => {
      await clearAllCache();

      expect(tauriFs.mock.remove).toHaveBeenCalledWith("attachment_cache", {
        baseDir: 26,
        recursive: true,
      });
    });

    it("clears cache in DB via invoke", async () => {
      await clearAllCache();

      expect(mockInvoke).toHaveBeenCalledWith("db_clear_all_attachment_cache");
    });
  });
});
