import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  type DbSendAsAlias,
  deleteAlias,
  getAliasesForAccount,
  getDefaultAlias,
  mapDbAlias,
  setDefaultAlias,
  upsertAlias,
} from "./sendAsAliases";

const mockInvoke = vi.mocked(invoke);

describe("sendAsAliases service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getAliasesForAccount", () => {
    it("calls invoke with correct command", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      await getAliasesForAccount("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith("db_get_aliases_for_account", {
        accountId: "acc-1",
      });
    });
  });

  describe("upsertAlias", () => {
    it("inserts an alias with correct parameters", async () => {
      mockInvoke.mockResolvedValueOnce("alias-id");

      await upsertAlias({
        accountId: "acc-1",
        email: "user@example.com",
        displayName: "User Name",
        isPrimary: true,
        isDefault: false,
        treatAsAlias: true,
        verificationStatus: "accepted",
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_alias", {
        accountId: "acc-1",
        email: "user@example.com",
        displayName: "User Name",
        replyToAddress: null,
        signatureId: null,
        isPrimary: true,
        isDefault: false,
        treatAsAlias: true,
        verificationStatus: "accepted",
      });
    });

    it("defaults treatAsAlias to true when not specified", async () => {
      mockInvoke.mockResolvedValueOnce("alias-id");

      await upsertAlias({
        accountId: "acc-1",
        email: "user@example.com",
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_alias", {
        accountId: "acc-1",
        email: "user@example.com",
        displayName: null,
        replyToAddress: null,
        signatureId: null,
        isPrimary: false,
        isDefault: false,
        treatAsAlias: true,
        verificationStatus: "accepted",
      });
    });
  });

  describe("getDefaultAlias", () => {
    it("returns the default alias when one exists", async () => {
      const alias: DbSendAsAlias = {
        id: "alias-1",
        account_id: "acc-1",
        email: "default@example.com",
        display_name: "Default",
        reply_to_address: null,
        signature_id: null,
        is_primary: 0,
        is_default: 1,
        treat_as_alias: 1,
        verification_status: "accepted",
        created_at: 1000,
      };
      mockInvoke.mockResolvedValueOnce(alias);

      const result = await getDefaultAlias("acc-1");

      expect(result).toEqual(alias);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_default_alias", {
        accountId: "acc-1",
      });
    });

    it("returns null when no aliases exist", async () => {
      mockInvoke.mockResolvedValueOnce(null);

      const result = await getDefaultAlias("acc-1");

      expect(result).toBeNull();
    });
  });

  describe("setDefaultAlias", () => {
    it("calls invoke with correct params", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await setDefaultAlias("acc-1", "alias-3");

      expect(mockInvoke).toHaveBeenCalledWith("db_set_default_alias", {
        accountId: "acc-1",
        aliasId: "alias-3",
      });
    });
  });

  describe("deleteAlias", () => {
    it("deletes the alias by id", async () => {
      mockInvoke.mockResolvedValueOnce(undefined);

      await deleteAlias("alias-5");

      expect(mockInvoke).toHaveBeenCalledWith("db_delete_alias", {
        id: "alias-5",
      });
    });
  });

  describe("mapDbAlias", () => {
    it("maps DB row to domain object", () => {
      const db: DbSendAsAlias = {
        id: "alias-1",
        account_id: "acc-1",
        email: "test@example.com",
        display_name: "Test User",
        reply_to_address: "reply@example.com",
        signature_id: "sig-1",
        is_primary: 1,
        is_default: 0,
        treat_as_alias: 1,
        verification_status: "accepted",
        created_at: 1700000000,
      };

      const result = mapDbAlias(db);

      expect(result).toEqual({
        id: "alias-1",
        accountId: "acc-1",
        email: "test@example.com",
        displayName: "Test User",
        replyToAddress: "reply@example.com",
        signatureId: "sig-1",
        isPrimary: true,
        isDefault: false,
        treatAsAlias: true,
        verificationStatus: "accepted",
      });
    });

    it("maps zero values to false booleans", () => {
      const db: DbSendAsAlias = {
        id: "alias-2",
        account_id: "acc-1",
        email: "test@example.com",
        display_name: null,
        reply_to_address: null,
        signature_id: null,
        is_primary: 0,
        is_default: 0,
        treat_as_alias: 0,
        verification_status: "pending",
        created_at: 1700000000,
      };

      const result = mapDbAlias(db);

      expect(result.isPrimary).toBe(false);
      expect(result.isDefault).toBe(false);
      expect(result.treatAsAlias).toBe(false);
      expect(result.displayName).toBeNull();
      expect(result.replyToAddress).toBeNull();
      expect(result.signatureId).toBeNull();
    });
  });
});
