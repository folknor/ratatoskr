import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  analyzeWritingStyle,
  generateAutoDraft,
  getOrCreateStyleProfile,
  isAutoDraftEnabled,
  refreshWritingStyle,
  regenerateAutoDraft,
} from "@/services/ai/writingStyleService";
import type { DbMessage } from "@/services/db/messages";

const { mockCompleteAi, mockTestAiConnection } = vi.hoisted(() => ({
  mockCompleteAi: vi.fn(),
  mockTestAiConnection: vi.fn(),
}));

vi.mock("@/services/ai/client", () => ({
  completeAi: mockCompleteAi,
  testAiConnection: mockTestAiConnection,
}));

vi.mock("@/services/db/aiCache", () => ({
  getAiCache: vi.fn().mockResolvedValue(null),
  setAiCache: vi.fn(),
  deleteAiCache: vi.fn(),
}));

vi.mock("@/services/db/writingStyleProfiles", () => ({
  getWritingStyleProfile: vi.fn().mockResolvedValue(null),
  upsertWritingStyleProfile: vi.fn(),
  deleteWritingStyleProfile: vi.fn(),
}));

vi.mock("@/services/db/messages", () => ({
  getRecentSentMessages: vi.fn().mockResolvedValue([]),
}));

vi.mock("@/services/accounts/basicInfo", () => ({
  getAccountBasicInfo: vi
    .fn()
    .mockResolvedValue({ id: "acc1", email: "user@example.com" }),
}));

vi.mock("@/services/db/settings", () => ({
  getSetting: vi.fn().mockResolvedValue("true"),
}));

const { getAiCache, setAiCache, deleteAiCache } = await import(
  "@/services/db/aiCache"
);
const {
  getWritingStyleProfile,
  upsertWritingStyleProfile,
  deleteWritingStyleProfile,
} = await import("@/services/db/writingStyleProfiles");
const { getRecentSentMessages } = await import("@/services/db/messages");
const { getAccountBasicInfo } = await import("@/services/accounts/basicInfo");
const { getSetting } = await import("@/services/db/settings");

function makeSentMessage(overrides: Partial<DbMessage> = {}): DbMessage {
  return {
    id: "msg1",
    account_id: "acc1",
    thread_id: "t1",
    from_address: "user@example.com",
    from_name: "User",
    to_addresses: "other@example.com",
    cc_addresses: null,
    bcc_addresses: null,
    reply_to: null,
    subject: "Test",
    snippet: "snippet",
    date: Date.now(),
    is_read: true,
    is_starred: false,
    body_html: "<p>Test body</p>",
    body_text:
      "Test body with enough content to be useful for analysis purposes here.",
    body_cached: true,
    raw_size: null,
    internal_date: null,
    list_unsubscribe: null,
    list_unsubscribe_post: null,
    auth_results: null,
    message_id_header: null,
    references_header: null,
    in_reply_to_header: null,
    imap_uid: null,
    imap_folder: null,
    ...overrides,
  };
}

describe("writingStyleService", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCompleteAi.mockResolvedValue("Mocked AI response");
    mockTestAiConnection.mockResolvedValue(true);
    vi.mocked(getAiCache).mockResolvedValue(null);
    vi.mocked(getWritingStyleProfile).mockResolvedValue(null);
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc1",
      email: "user@example.com",
      provider: "gmail_api",
      isActive: true,
    });
    vi.mocked(getSetting).mockResolvedValue("true");
    vi.mocked(getRecentSentMessages).mockResolvedValue([]);
  });

  describe("analyzeWritingStyle", () => {
    it("calls AI with formatted samples", async () => {
      const samples = [makeSentMessage(), makeSentMessage({ id: "msg2" })];

      const result = await analyzeWritingStyle(samples);

      expect(result).toBe("Mocked AI response");
      expect(mockCompleteAi).toHaveBeenCalledTimes(1);
    });
  });

  describe("getOrCreateStyleProfile", () => {
    it("returns existing profile if cached", async () => {
      vi.mocked(getWritingStyleProfile).mockResolvedValue({
        id: "p1",
        account_id: "acc1",
        profile_text: "Formal tone",
        sample_count: 10,
        created_at: 1000,
        updated_at: 1000,
      });

      const result = await getOrCreateStyleProfile("acc1");

      expect(result).toBe("Formal tone");
    });

    it("returns null when style learning is disabled", async () => {
      vi.mocked(getSetting).mockResolvedValue("false");

      const result = await getOrCreateStyleProfile("acc1");

      expect(result).toBeNull();
    });

    it("returns null when less than 3 sent messages", async () => {
      vi.mocked(getRecentSentMessages).mockResolvedValue([makeSentMessage()]);

      const result = await getOrCreateStyleProfile("acc1");

      expect(result).toBeNull();
    });

    it("creates profile from sent messages when none exists", async () => {
      const msgs = Array.from({ length: 5 }, (_, i) =>
        makeSentMessage({ id: `msg${i}` }),
      );
      vi.mocked(getRecentSentMessages).mockResolvedValue(msgs);

      const result = await getOrCreateStyleProfile("acc1");

      expect(result).toBe("Mocked AI response");
      expect(upsertWritingStyleProfile).toHaveBeenCalledWith(
        "acc1",
        "Mocked AI response",
        5,
      );
    });
  });

  describe("refreshWritingStyle", () => {
    it("deletes existing profile and recreates", async () => {
      const msgs = Array.from({ length: 5 }, (_, i) =>
        makeSentMessage({ id: `msg${i}` }),
      );
      vi.mocked(getRecentSentMessages).mockResolvedValue(msgs);

      await refreshWritingStyle("acc1");

      expect(deleteWritingStyleProfile).toHaveBeenCalledWith("acc1");
    });
  });

  describe("generateAutoDraft", () => {
    const msgs = [
      makeSentMessage({ from_address: "other@test.com", from_name: "Other" }),
    ];

    it("returns cached draft if available", async () => {
      vi.mocked(getAiCache).mockResolvedValue("<p>Cached draft</p>");

      const result = await generateAutoDraft("t1", "acc1", msgs, "reply");

      expect(result).toBe("<p>Cached draft</p>");
    });

    it("generates and caches new draft", async () => {
      const result = await generateAutoDraft("t1", "acc1", msgs, "reply");

      expect(result).toBe("Mocked AI response");
      expect(setAiCache).toHaveBeenCalledWith(
        "acc1",
        "t1",
        "auto_draft_reply",
        "Mocked AI response",
      );
    });

    it("uses correct cache type for replyAll", async () => {
      await generateAutoDraft("t1", "acc1", msgs, "replyAll");

      expect(getAiCache).toHaveBeenCalledWith(
        "acc1",
        "t1",
        "auto_draft_replyAll",
      );
    });
  });

  describe("regenerateAutoDraft", () => {
    it("clears cache before generating", async () => {
      const msgs = [makeSentMessage()];

      await regenerateAutoDraft("t1", "acc1", msgs, "reply");

      expect(deleteAiCache).toHaveBeenCalledWith(
        "acc1",
        "t1",
        "auto_draft_reply",
      );
    });
  });

  describe("isAutoDraftEnabled", () => {
    it("returns false when setting is disabled", async () => {
      vi.mocked(getSetting).mockResolvedValue("false");

      const result = await isAutoDraftEnabled();

      expect(result).toBe(false);
    });

    it("returns true when AI is configured and enabled", async () => {
      const result = await isAutoDraftEnabled();

      expect(result).toBe(true);
      expect(mockTestAiConnection).toHaveBeenCalledTimes(1);
    });
  });
});
