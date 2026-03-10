import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteQuickStep,
  getEnabledQuickStepsForAccount,
  getQuickStepsForAccount,
  insertQuickStep,
  reorderQuickSteps,
  updateQuickStep,
} from "./quickSteps";

describe("quickSteps DB service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getQuickStepsForAccount", () => {
    it("queries all quick steps for an account", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([]);

      await getQuickStepsForAccount("acct-1");

      expect(invoke).toHaveBeenCalledWith("db_get_quick_steps_for_account", {
        accountId: "acct-1",
      });
    });
  });

  describe("getEnabledQuickStepsForAccount", () => {
    it("queries only enabled quick steps", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([]);

      await getEnabledQuickStepsForAccount("acct-1");

      expect(invoke).toHaveBeenCalledWith(
        "db_get_enabled_quick_steps_for_account",
        { accountId: "acct-1" },
      );
    });
  });

  describe("insertQuickStep", () => {
    it("inserts a quick step with serialized actions JSON", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);
      const actions = [
        { type: "archive" as const },
        { type: "markRead" as const },
      ];

      const id = await insertQuickStep({
        accountId: "acct-1",
        name: "Test Step",
        actions,
      });

      expect(id).toBeTruthy();
      expect(invoke).toHaveBeenCalledWith("db_insert_quick_step", {
        step: {
          id: expect.any(String),
          account_id: "acct-1",
          name: "Test Step",
          description: null,
          shortcut: null,
          actions_json: JSON.stringify(actions),
          icon: null,
          is_enabled: true,
          continue_on_error: false,
          sort_order: 0,
          created_at: 0,
        },
      });
    });

    it("passes optional fields when provided", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await insertQuickStep({
        accountId: "acct-1",
        name: "Custom Step",
        description: "A test description",
        shortcut: "Ctrl+1",
        actions: [{ type: "star" as const }],
        icon: "Star",
        isEnabled: false,
        continueOnError: true,
      });

      expect(invoke).toHaveBeenCalledWith("db_insert_quick_step", {
        step: expect.objectContaining({
          description: "A test description",
          shortcut: "Ctrl+1",
          icon: "Star",
          is_enabled: false,
          continue_on_error: true,
        }),
      });
    });
  });

  describe("updateQuickStep", () => {
    it("calls invoke with merged step struct", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);
      const actions = [{ type: "trash" as const }];

      await updateQuickStep("qs-1", {
        name: "New Name",
        actions,
        isEnabled: true,
        continueOnError: false,
      });

      expect(invoke).toHaveBeenCalledWith("db_update_quick_step", {
        step: expect.objectContaining({
          id: "qs-1",
          name: "New Name",
          actions_json: JSON.stringify(actions),
          is_enabled: true,
          continue_on_error: false,
        }),
      });
    });
  });

  describe("deleteQuickStep", () => {
    it("deletes by id", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await deleteQuickStep("qs-1");

      expect(invoke).toHaveBeenCalledWith("db_delete_quick_step", {
        id: "qs-1",
      });
    });
  });

  describe("reorderQuickSteps", () => {
    it("updates sort_order for each id in order", async () => {
      const steps = [
        {
          id: "qs-a",
          account_id: "acct-1",
          name: "A",
          description: null,
          shortcut: null,
          actions_json: "[]",
          icon: null,
          is_enabled: true,
          continue_on_error: false,
          sort_order: 0,
          created_at: 100,
        },
        {
          id: "qs-b",
          account_id: "acct-1",
          name: "B",
          description: null,
          shortcut: null,
          actions_json: "[]",
          icon: null,
          is_enabled: true,
          continue_on_error: false,
          sort_order: 1,
          created_at: 200,
        },
        {
          id: "qs-c",
          account_id: "acct-1",
          name: "C",
          description: null,
          shortcut: null,
          actions_json: "[]",
          icon: null,
          is_enabled: true,
          continue_on_error: false,
          sort_order: 2,
          created_at: 300,
        },
      ];
      // First call returns the steps, subsequent calls are updates
      vi.mocked(invoke)
        .mockResolvedValueOnce(steps)
        .mockResolvedValue(undefined);

      await reorderQuickSteps("acct-1", ["qs-b", "qs-a", "qs-c"]);

      // First call fetches steps
      expect(invoke).toHaveBeenCalledWith("db_get_quick_steps_for_account", {
        accountId: "acct-1",
      });
      // Then 3 updates
      expect(invoke).toHaveBeenCalledWith("db_update_quick_step", {
        step: expect.objectContaining({ id: "qs-b", sort_order: 0 }),
      });
      expect(invoke).toHaveBeenCalledWith("db_update_quick_step", {
        step: expect.objectContaining({ id: "qs-a", sort_order: 1 }),
      });
      expect(invoke).toHaveBeenCalledWith("db_update_quick_step", {
        step: expect.objectContaining({ id: "qs-c", sort_order: 2 }),
      });
    });
  });
});
