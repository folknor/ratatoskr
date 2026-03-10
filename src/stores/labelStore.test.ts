import { beforeEach, describe, expect, it, vi } from "vitest";
import { isSystemLabel, useLabelStore } from "./labelStore";

vi.mock("@/services/db/labels", () => ({
  getLabelsForAccount: vi.fn(),
  deleteLabel: vi.fn(),
  updateLabelSortOrder: vi.fn(),
  upsertLabel: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteLabel as dbDeleteLabel,
  getLabelsForAccount,
  updateLabelSortOrder,
  upsertLabel,
} from "@/services/db/labels";

const mockGetLabels = vi.mocked(getLabelsForAccount);
const mockDbDeleteLabel = vi.mocked(dbDeleteLabel);
const mockUpdateSortOrder = vi.mocked(updateLabelSortOrder);
const mockUpsertLabel = vi.mocked(upsertLabel);
const mockInvoke = vi.mocked(invoke);

describe("labelStore", () => {
  beforeEach(() => {
    useLabelStore.setState({ labels: [], isLoading: false });
    vi.clearAllMocks();
  });

  it("should have correct default state", () => {
    const state = useLabelStore.getState();
    expect(state.labels).toEqual([]);
    expect(state.isLoading).toBe(false);
  });

  it("should clear labels", () => {
    useLabelStore.setState({
      labels: [
        {
          id: "Label_1",
          accountId: "acc1",
          name: "Work",
          type: "user",
          colorBg: null,
          colorFg: null,
          sortOrder: 0,
        },
      ],
      isLoading: true,
    });
    useLabelStore.getState().clearLabels();
    const state = useLabelStore.getState();
    expect(state.labels).toEqual([]);
    expect(state.isLoading).toBe(false);
  });

  it("should load labels and filter out system labels", async () => {
    mockGetLabels.mockResolvedValue([
      {
        id: "INBOX",
        account_id: "acc1",
        name: "INBOX",
        type: "system",
        color_bg: null,
        color_fg: null,
        visible: 1,
        sort_order: 0,
      },
      {
        id: "SENT",
        account_id: "acc1",
        name: "SENT",
        type: "system",
        color_bg: null,
        color_fg: null,
        visible: 1,
        sort_order: 1,
      },
      {
        id: "CATEGORY_SOCIAL",
        account_id: "acc1",
        name: "Social",
        type: "system",
        color_bg: null,
        color_fg: null,
        visible: 1,
        sort_order: 2,
      },
      {
        id: "Label_1",
        account_id: "acc1",
        name: "Work",
        type: "user",
        color_bg: "#4285f4",
        color_fg: "#ffffff",
        visible: 1,
        sort_order: 3,
      },
      {
        id: "Label_2",
        account_id: "acc1",
        name: "Personal",
        type: "user",
        color_bg: null,
        color_fg: null,
        visible: 1,
        sort_order: 4,
      },
    ]);

    await useLabelStore.getState().loadLabels("acc1");

    const state = useLabelStore.getState();
    expect(state.labels).toHaveLength(2);
    expect(state.labels[0]).toEqual({
      id: "Label_1",
      accountId: "acc1",
      name: "Work",
      type: "user",
      colorBg: "#4285f4",
      colorFg: "#ffffff",
      sortOrder: 3,
    });
    expect(state.labels[1]).toEqual({
      id: "Label_2",
      accountId: "acc1",
      name: "Personal",
      type: "user",
      colorBg: null,
      colorFg: null,
      sortOrder: 4,
    });
    expect(state.isLoading).toBe(false);
  });

  it("should handle load error gracefully", async () => {
    mockGetLabels.mockRejectedValue(new Error("DB error"));
    await useLabelStore.getState().loadLabels("acc1");
    const state = useLabelStore.getState();
    expect(state.labels).toEqual([]);
    expect(state.isLoading).toBe(false);
  });

  it("should create a label via Rust command and update DB", async () => {
    mockInvoke.mockResolvedValue({
      id: "Label_new",
      name: "New Label",
      type: "user",
      color: { backgroundColor: "#fb4c2f", textColor: "#ffffff" },
    });
    mockUpsertLabel.mockResolvedValue(undefined);
    mockGetLabels.mockResolvedValue([]);

    await useLabelStore.getState().createLabel("acc1", "New Label", {
      textColor: "#ffffff",
      backgroundColor: "#fb4c2f",
    });

    expect(mockInvoke).toHaveBeenCalledWith("gmail_create_label", {
      accountId: "acc1",
      name: "New Label",
      textColor: "#ffffff",
      bgColor: "#fb4c2f",
    });
    expect(mockUpsertLabel).toHaveBeenCalledWith({
      id: "Label_new",
      accountId: "acc1",
      name: "New Label",
      type: "user",
      colorBg: "#fb4c2f",
      colorFg: "#ffffff",
    });
    expect(mockGetLabels).toHaveBeenCalledWith("acc1");
  });

  it("should update a label via Rust command and update DB", async () => {
    mockInvoke.mockResolvedValue({
      id: "Label_1",
      name: "Renamed",
      type: "user",
      color: { backgroundColor: "#16a765", textColor: "#ffffff" },
    });
    mockUpsertLabel.mockResolvedValue(undefined);
    mockGetLabels.mockResolvedValue([]);

    await useLabelStore.getState().updateLabel("acc1", "Label_1", {
      name: "Renamed",
      color: { textColor: "#ffffff", backgroundColor: "#16a765" },
    });

    expect(mockInvoke).toHaveBeenCalledWith("gmail_update_label", {
      accountId: "acc1",
      labelId: "Label_1",
      name: "Renamed",
      textColor: "#ffffff",
      bgColor: "#16a765",
    });
    expect(mockUpsertLabel).toHaveBeenCalled();
  });

  it("should delete a label via Rust command and DB", async () => {
    mockInvoke.mockResolvedValue(undefined);
    mockDbDeleteLabel.mockResolvedValue(undefined);
    mockGetLabels.mockResolvedValue([]);

    await useLabelStore.getState().deleteLabel("acc1", "Label_1");

    expect(mockInvoke).toHaveBeenCalledWith("gmail_delete_label", {
      accountId: "acc1",
      labelId: "Label_1",
    });
    expect(mockDbDeleteLabel).toHaveBeenCalledWith("acc1", "Label_1");
    expect(mockGetLabels).toHaveBeenCalledWith("acc1");
  });

  it("should reorder labels by updating sort order in DB", async () => {
    mockUpdateSortOrder.mockResolvedValue(undefined);
    mockGetLabels.mockResolvedValue([]);

    await useLabelStore
      .getState()
      .reorderLabels("acc1", ["Label_2", "Label_1", "Label_3"]);

    expect(mockUpdateSortOrder).toHaveBeenCalledWith("acc1", [
      { id: "Label_2", sortOrder: 0 },
      { id: "Label_1", sortOrder: 1 },
      { id: "Label_3", sortOrder: 2 },
    ]);
    expect(mockGetLabels).toHaveBeenCalledWith("acc1");
  });
});

describe("isSystemLabel", () => {
  it("should identify system labels", () => {
    expect(isSystemLabel("INBOX")).toBe(true);
    expect(isSystemLabel("SENT")).toBe(true);
    expect(isSystemLabel("DRAFT")).toBe(true);
    expect(isSystemLabel("TRASH")).toBe(true);
    expect(isSystemLabel("SPAM")).toBe(true);
    expect(isSystemLabel("STARRED")).toBe(true);
    expect(isSystemLabel("UNREAD")).toBe(true);
    expect(isSystemLabel("IMPORTANT")).toBe(true);
    expect(isSystemLabel("SNOOZED")).toBe(true);
    expect(isSystemLabel("CHAT")).toBe(true);
  });

  it("should identify category labels as system labels", () => {
    expect(isSystemLabel("CATEGORY_SOCIAL")).toBe(true);
    expect(isSystemLabel("CATEGORY_UPDATES")).toBe(true);
    expect(isSystemLabel("CATEGORY_PROMOTIONS")).toBe(true);
  });

  it("should not flag user labels as system labels", () => {
    expect(isSystemLabel("Label_1")).toBe(false);
    expect(isSystemLabel("Label_2")).toBe(false);
    expect(isSystemLabel("Work")).toBe(false);
  });
});
