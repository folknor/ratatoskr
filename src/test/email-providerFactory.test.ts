import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/services/accounts/basicInfo", () => ({
  getAccountBasicInfo: vi.fn(),
}));

import { getAccountBasicInfo } from "@/services/accounts/basicInfo";
import { GmailApiProvider } from "@/services/email/gmailProvider";
import { GraphProvider } from "@/services/email/graphProvider";
import { ImapSmtpProvider } from "@/services/email/imapSmtpProvider";
import { JmapProvider } from "@/services/email/jmapProvider";
import {
  clearAllProviders,
  getEmailProvider,
  invalidateProviderConfig,
  removeProvider,
} from "@/services/email/providerFactory";

describe("providerFactory", () => {
  beforeEach(() => {
    clearAllProviders();
    vi.clearAllMocks();
  });

  it("returns GmailApiProvider for gmail_api accounts", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-1",
      email: "user@gmail.com",
      displayName: null,
      avatarUrl: null,
      provider: "gmail_api",
      isActive: true,
    });

    const provider = await getEmailProvider("acc-1");

    expect(provider).toBeInstanceOf(GmailApiProvider);
    expect(provider.accountId).toBe("acc-1");
    expect(provider.type).toBe("gmail_api");
  });

  it("returns ImapSmtpProvider for imap accounts", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-2",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "imap",
      isActive: true,
    });

    const provider = await getEmailProvider("acc-2");

    expect(provider).toBeInstanceOf(ImapSmtpProvider);
    expect(provider.accountId).toBe("acc-2");
    expect(provider.type).toBe("imap");
  });

  it("returns JmapProvider for jmap accounts", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-jmap",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "jmap",
      isActive: true,
    });

    const provider = await getEmailProvider("acc-jmap");

    expect(provider).toBeInstanceOf(JmapProvider);
    expect(provider.accountId).toBe("acc-jmap");
    expect(provider.type).toBe("jmap");
  });

  it("returns GraphProvider for graph accounts", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-graph",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "graph",
      isActive: true,
    });

    const provider = await getEmailProvider("acc-graph");

    expect(provider).toBeInstanceOf(GraphProvider);
    expect(provider.accountId).toBe("acc-graph");
    expect(provider.type).toBe("graph");
  });

  it("caches providers and returns same instance", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-3",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "imap",
      isActive: true,
    });

    const first = await getEmailProvider("acc-3");
    const second = await getEmailProvider("acc-3");

    expect(first).toBe(second);
    expect(getAccountBasicInfo).toHaveBeenCalledTimes(1);
  });

  it("removeProvider evicts from cache", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-4",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "imap",
      isActive: true,
    });

    const first = await getEmailProvider("acc-4");
    removeProvider("acc-4");
    const second = await getEmailProvider("acc-4");

    expect(first).not.toBe(second);
    expect(getAccountBasicInfo).toHaveBeenCalledTimes(2);
  });

  it("clearAllProviders empties the cache", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-5",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "imap",
      isActive: true,
    });

    const first = await getEmailProvider("acc-5");
    clearAllProviders();
    const second = await getEmailProvider("acc-5");

    expect(first).not.toBe(second);
    expect(getAccountBasicInfo).toHaveBeenCalledTimes(2);
  });

  it("throws when account is not found", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue(null);

    await expect(getEmailProvider("nonexistent")).rejects.toThrow(
      "Account nonexistent not found",
    );
  });

  it("invalidateProviderConfig clears IMAP config cache", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-6",
      email: "user@example.com",
      displayName: null,
      avatarUrl: null,
      provider: "imap",
      isActive: true,
    });

    const provider = await getEmailProvider("acc-6");
    expect(provider).toBeInstanceOf(ImapSmtpProvider);

    const clearSpy = vi.spyOn(provider as ImapSmtpProvider, "clearConfigCache");

    invalidateProviderConfig("acc-6");

    expect(clearSpy).toHaveBeenCalledTimes(1);
  });

  it("invalidateProviderConfig is a no-op for uncached accounts", () => {
    invalidateProviderConfig("nonexistent-account");
  });

  it("invalidateProviderConfig is a no-op for Gmail providers", async () => {
    vi.mocked(getAccountBasicInfo).mockResolvedValue({
      id: "acc-7",
      email: "user@gmail.com",
      displayName: null,
      avatarUrl: null,
      provider: "gmail_api",
      isActive: true,
    });

    await getEmailProvider("acc-7");

    invalidateProviderConfig("acc-7");
  });
});
