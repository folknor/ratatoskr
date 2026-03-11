import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@/services/smartLabels/smartLabelService", () => ({
  matchSmartLabels: vi.fn(),
  classifySmartLabelRemainder: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "@/services/gmail/messageParser";
import {
  applySmartLabelsFromAiRemainder,
  applySmartLabelsToMessages,
} from "@/services/smartLabels/smartLabelManager";
import {
  classifySmartLabelRemainder,
  matchSmartLabels,
} from "@/services/smartLabels/smartLabelService";

function makeMessage(threadId = "t1"): ParsedMessage {
  return {
    id: `msg-${threadId}`,
    threadId,
    fromAddress: "sender@example.com",
    fromName: "Sender",
    toAddresses: "me@example.com",
    ccAddresses: null,
    bccAddresses: null,
    replyTo: null,
    subject: "Test",
    snippet: "Test",
    date: Date.now(),
    isRead: false,
    isStarred: false,
    bodyHtml: null,
    bodyText: null,
    rawSize: 0,
    internalDate: Date.now(),
    labelIds: [],
    hasAttachments: false,
    attachments: [],
    listUnsubscribe: null,
    listUnsubscribePost: null,
    authResults: null,
  };
}

describe("applySmartLabelsToMessages", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("applies matched labels via rust invoke", async () => {
    vi.mocked(matchSmartLabels).mockResolvedValue([
      { threadId: "t1", labelIds: ["label-a", "label-b"] },
      { threadId: "t2", labelIds: ["label-c"] },
    ]);

    await applySmartLabelsToMessages("acc-1", "gmail_api", [
      makeMessage("t1"),
      makeMessage("t2"),
    ]);

    expect(invoke).toHaveBeenCalledWith("smart_labels_apply_matches", {
      accountId: "acc-1",
      provider: "gmail_api",
      matches: [
        { threadId: "t1", labelIds: ["label-a", "label-b"] },
        { threadId: "t2", labelIds: ["label-c"] },
      ],
    });
  });

  it("does not throw when matchSmartLabels returns empty", async () => {
    vi.mocked(matchSmartLabels).mockResolvedValue([]);

    await expect(
      applySmartLabelsToMessages("acc-1", "gmail_api", [makeMessage()]),
    ).resolves.toBeUndefined();

    expect(invoke).not.toHaveBeenCalled();
  });

  it("does not throw when matchSmartLabels fails", async () => {
    vi.mocked(matchSmartLabels).mockRejectedValue(new Error("DB error"));

    await expect(
      applySmartLabelsToMessages("acc-1", "gmail_api", [makeMessage()]),
    ).resolves.toBeUndefined();
  });

  it("does not throw when rust label application fails", async () => {
    vi.mocked(matchSmartLabels).mockResolvedValue([
      { threadId: "t1", labelIds: ["label-a"] },
    ]);
    vi.mocked(invoke).mockRejectedValue(new Error("IPC error"));

    await expect(
      applySmartLabelsToMessages("acc-1", "gmail_api", [makeMessage()]),
    ).resolves.toBeUndefined();
  });

  it("passes pre-applied matches into ai remainder classification", async () => {
    vi.mocked(classifySmartLabelRemainder).mockResolvedValue([
      { threadId: "t1", labelIds: ["label-b"] },
    ]);

    await applySmartLabelsFromAiRemainder(
      "acc-1",
      "gmail_api",
      [{ threadId: "t1", labelIds: ["label-a"] }],
      {
        threads: [
          {
            id: "t1",
            subject: "Test",
            snippet: "Snippet",
            fromAddress: "sender@example.com",
          },
        ],
        rules: [{ labelId: "label-b", description: "B" }],
      },
    );

    expect(classifySmartLabelRemainder).toHaveBeenCalledWith(
      [
        {
          id: "t1",
          subject: "Test",
          snippet: "Snippet",
          fromAddress: "sender@example.com",
        },
      ],
      [{ labelId: "label-b", description: "B" }],
      [{ threadId: "t1", labelIds: ["label-a"] }],
    );
  });
});
