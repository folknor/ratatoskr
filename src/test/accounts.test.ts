import { vi } from "vitest";
import { createMockGmailAccount, createMockImapAccount } from "@/test/mocks";
import { decryptValue } from "@/utils/crypto";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@/utils/crypto", () => ({
  encryptValue: vi.fn((val: string) => Promise.resolve(`enc:${val}`)),
  decryptValue: vi.fn((val: string) =>
    Promise.resolve(val.replace("enc:", "")),
  ),
  isEncrypted: vi.fn((val: string) => val.startsWith("enc:")),
}));

import {
  deleteAccount,
  getAccount,
  getAccountByEmail,
  getAllAccounts,
  insertAccount,
  insertImapAccount,
  updateAccountSyncState,
  updateAccountTokens,
} from "@/services/db/accounts";

describe("accounts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("getAccount", () => {
    it("returns null for non-existent account", async () => {
      mockInvoke.mockResolvedValue(null);

      const result = await getAccount("nonexistent");

      expect(result).toBeNull();
      expect(mockInvoke).toHaveBeenCalledWith("db_get_account", {
        id: "nonexistent",
      });
    });

    it("returns a Gmail account with decrypted tokens", async () => {
      mockInvoke.mockResolvedValue(createMockGmailAccount());

      const result = await getAccount("acc-gmail");

      expect(result).not.toBeNull();
      expect(result?.id).toBe("acc-gmail");
      expect(result?.provider).toBe("gmail_api");
      expect(result?.access_token).toBe("access-token");
      expect(result?.refresh_token).toBe("refresh-token");
    });

    it("returns an IMAP account with decrypted imap_password", async () => {
      mockInvoke.mockResolvedValue(createMockImapAccount());

      const result = await getAccount("acc-imap");

      expect(result).not.toBeNull();
      expect(result?.provider).toBe("imap");
      expect(result?.imap_host).toBe("imap.example.com");
      expect(result?.imap_port).toBe(993);
      expect(result?.imap_security).toBe("tls");
      expect(result?.smtp_host).toBe("smtp.example.com");
      expect(result?.smtp_port).toBe(465);
      expect(result?.smtp_security).toBe("tls");
      expect(result?.auth_method).toBe("password");
      expect(result?.imap_password).toBe("secret-password");
    });

    it("handles IMAP account with null imap_password gracefully", async () => {
      mockInvoke.mockResolvedValue(
        createMockImapAccount({ imap_password: null }),
      );

      const result = await getAccount("acc-imap");

      expect(result?.imap_password).toBeNull();
    });

    it("clears encrypted secrets when decryption fails", async () => {
      const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
      mockInvoke.mockResolvedValue(createMockImapAccount());
      vi.mocked(decryptValue).mockRejectedValueOnce(new Error("bad key"));

      const result = await getAccount("acc-imap");

      expect(result?.imap_password).toBeNull();
      expect(warnSpy).toHaveBeenCalled();
    });
  });

  describe("getAccountByEmail", () => {
    it("returns account matching email", async () => {
      mockInvoke.mockResolvedValue(createMockImapAccount());

      const result = await getAccountByEmail("user@example.com");

      expect(result).not.toBeNull();
      expect(result?.email).toBe("user@example.com");
      expect(mockInvoke).toHaveBeenCalledWith("db_get_account_by_email", {
        email: "user@example.com",
      });
    });

    it("returns null when email not found", async () => {
      mockInvoke.mockResolvedValue(null);

      const result = await getAccountByEmail("unknown@example.com");

      expect(result).toBeNull();
    });
  });

  describe("getAllAccounts", () => {
    it("returns all accounts with decrypted tokens", async () => {
      mockInvoke.mockResolvedValue([
        createMockGmailAccount(),
        createMockImapAccount(),
      ]);

      const result = await getAllAccounts();

      expect(result).toHaveLength(2);
      expect(result[0]?.provider).toBe("gmail_api");
      expect(result[0]?.access_token).toBe("access-token");
      expect(result[1]?.provider).toBe("imap");
      expect(result[1]?.imap_password).toBe("secret-password");
    });

    it("returns empty array when no accounts exist", async () => {
      mockInvoke.mockResolvedValue([]);

      const result = await getAllAccounts();

      expect(result).toEqual([]);
    });

    it("decrypts imap_password for IMAP accounts in the list", async () => {
      mockInvoke.mockResolvedValue([createMockImapAccount()]);

      const result = await getAllAccounts();

      expect(result[0]?.imap_password).toBe("secret-password");
    });
  });

  describe("insertImapAccount", () => {
    it("inserts IMAP account with encrypted password", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await insertImapAccount({
        id: "new-imap",
        email: "user@fastmail.com",
        displayName: "Fastmail User",
        avatarUrl: null,
        imapHost: "imap.fastmail.com",
        imapPort: 993,
        imapSecurity: "ssl",
        smtpHost: "smtp.fastmail.com",
        smtpPort: 465,
        smtpSecurity: "ssl",
        authMethod: "password",
        password: "my-app-password",
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_insert_account", {
        id: "new-imap",
        email: "user@fastmail.com",
        displayName: "Fastmail User",
        avatarUrl: null,
        provider: "imap",
        authMethod: "password",
        imapHost: "imap.fastmail.com",
        imapPort: 993,
        imapSecurity: "ssl",
        smtpHost: "smtp.fastmail.com",
        smtpPort: 465,
        smtpSecurity: "ssl",
        imapPassword: "enc:my-app-password",
        imapUsername: null,
        acceptInvalidCerts: 0,
      });
    });

    it("inserts IMAP account with custom username", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await insertImapAccount({
        id: "new-imap-2",
        email: "user@example.com",
        displayName: null,
        avatarUrl: null,
        imapHost: "imap.example.com",
        imapPort: 993,
        imapSecurity: "ssl",
        smtpHost: "smtp.example.com",
        smtpPort: 465,
        smtpSecurity: "ssl",
        authMethod: "password",
        password: "pass",
        imapUsername: "custom-login-id",
      });

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_insert_account",
        expect.objectContaining({
          imapUsername: "custom-login-id",
        }),
      );
    });
  });

  describe("insertAccount (Gmail/OAuth)", () => {
    it("inserts OAuth account with encrypted tokens", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await insertAccount({
        id: "gmail-1",
        email: "user@gmail.com",
        displayName: "Test User",
        avatarUrl: "https://example.com/avatar.jpg",
        accessToken: "access-token-123",
        refreshToken: "refresh-token-456",
        tokenExpiresAt: 9999999999,
      });

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_insert_account",
        expect.objectContaining({
          accessToken: "enc:access-token-123",
          refreshToken: "enc:refresh-token-456",
          provider: "gmail_api",
          authMethod: "oauth2",
        }),
      );
    });
  });

  describe("deleteAccount", () => {
    it("deletes account by id", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await deleteAccount("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith("account_delete", {
        accountId: "acc-1",
      });
    });
  });

  describe("updateAccountTokens", () => {
    it("updates access_token with encryption", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await updateAccountTokens("acc-1", "new-token", 1234567890);

      expect(mockInvoke).toHaveBeenCalledWith("db_update_account_tokens", {
        id: "acc-1",
        accessToken: "enc:new-token",
        tokenExpiresAt: 1234567890,
      });
    });
  });

  describe("updateAccountSyncState", () => {
    it("updates history_id and last_sync_at", async () => {
      mockInvoke.mockResolvedValue(undefined);

      await updateAccountSyncState("acc-1", "history-999");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_account_sync_state", {
        id: "acc-1",
        historyId: "history-999",
      });
    });
  });
});
