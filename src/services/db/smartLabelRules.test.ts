import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  deleteSmartLabelRule,
  getEnabledSmartLabelRules,
  getSmartLabelRulesForAccount,
  insertSmartLabelRule,
  updateSmartLabelRule,
} from "./smartLabelRules";

describe("smartLabelRules service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getSmartLabelRulesForAccount", () => {
    it("returns rules for the account", async () => {
      const mockRules = [
        {
          id: "r1",
          account_id: "acc-1",
          label_id: "l1",
          ai_description: "Test",
          criteria_json: null,
          is_enabled: true,
          sort_order: 0,
          created_at: 100,
        },
      ];
      vi.mocked(invoke).mockResolvedValueOnce(mockRules);

      const result = await getSmartLabelRulesForAccount("acc-1");

      expect(result).toEqual(mockRules);
      expect(invoke).toHaveBeenCalledWith(
        "db_get_smart_label_rules_for_account",
        { accountId: "acc-1" },
      );
    });
  });

  describe("getEnabledSmartLabelRules", () => {
    it("returns only enabled rules", async () => {
      vi.mocked(invoke).mockResolvedValueOnce([
        {
          id: "r1",
          account_id: "acc-1",
          label_id: "l1",
          ai_description: "Enabled",
          criteria_json: null,
          is_enabled: true,
          sort_order: 0,
          created_at: 100,
        },
        {
          id: "r2",
          account_id: "acc-1",
          label_id: "l2",
          ai_description: "Disabled",
          criteria_json: null,
          is_enabled: false,
          sort_order: 1,
          created_at: 200,
        },
      ]);

      const result = await getEnabledSmartLabelRules("acc-1");

      expect(result).toHaveLength(1);
      expect(result[0]?.id).toBe("r1");
    });
  });

  describe("insertSmartLabelRule", () => {
    it("inserts with required fields", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      const id = await insertSmartLabelRule({
        accountId: "acc-1",
        labelId: "label-1",
        aiDescription: "Job applications",
      });

      expect(id).toBeTruthy();
      expect(invoke).toHaveBeenCalledWith("db_insert_smart_label_rule", {
        id: expect.any(String),
        accountId: "acc-1",
        labelId: "label-1",
        aiDescription: "Job applications",
        criteriaJson: null,
        isEnabled: true,
      });
    });

    it("inserts with optional criteria", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await insertSmartLabelRule({
        accountId: "acc-1",
        labelId: "label-1",
        aiDescription: "Job apps",
        criteria: { from: "recruiter@", subject: "position" },
      });

      expect(invoke).toHaveBeenCalledWith("db_insert_smart_label_rule", {
        id: expect.any(String),
        accountId: "acc-1",
        labelId: "label-1",
        aiDescription: "Job apps",
        criteriaJson: JSON.stringify({
          from: "recruiter@",
          subject: "position",
        }),
        isEnabled: true,
      });
    });

    it("inserts as disabled when isEnabled is false", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await insertSmartLabelRule({
        accountId: "acc-1",
        labelId: "label-1",
        aiDescription: "Test",
        isEnabled: false,
      });

      expect(invoke).toHaveBeenCalledWith(
        "db_insert_smart_label_rule",
        expect.objectContaining({ isEnabled: false }),
      );
    });
  });

  describe("updateSmartLabelRule", () => {
    it("passes updates via invoke", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await updateSmartLabelRule("r1", {
        aiDescription: "Updated description",
      });

      expect(invoke).toHaveBeenCalledWith("db_update_smart_label_rule", {
        id: "r1",
        aiDescription: "Updated description",
      });
    });

    it("serializes criteria to JSON", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await updateSmartLabelRule("r1", {
        criteria: { from: "test@example.com" },
      });

      expect(invoke).toHaveBeenCalledWith("db_update_smart_label_rule", {
        id: "r1",
        criteriaJson: JSON.stringify({ from: "test@example.com" }),
      });
    });

    it("clears criteria when set to null", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await updateSmartLabelRule("r1", { criteria: null });

      expect(invoke).toHaveBeenCalledWith("db_update_smart_label_rule", {
        id: "r1",
        criteriaJson: null,
      });
    });
  });

  describe("deleteSmartLabelRule", () => {
    it("deletes by id", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(undefined);

      await deleteSmartLabelRule("r1");

      expect(invoke).toHaveBeenCalledWith("db_delete_smart_label_rule", {
        id: "r1",
      });
    });
  });
});
