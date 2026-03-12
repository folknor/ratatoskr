import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockInvoke, mockListAccountBasicInfo } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  mockListAccountBasicInfo: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mockInvoke,
}));

vi.mock("@/services/accounts/basicInfo", () => ({
  listAccountBasicInfo: mockListAccountBasicInfo,
}));

async function loadTokenManager() {
  return import("@/services/gmail/tokenManager");
}

describe("tokenManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("initializes only active gmail accounts", async () => {
    mockListAccountBasicInfo.mockResolvedValue([
      {
        id: "gmail-active",
        email: "user@gmail.com",
        displayName: "User",
        avatarUrl: null,
        provider: "gmail_api",
        isActive: true,
      },
      {
        id: "gmail-inactive",
        email: "inactive@gmail.com",
        displayName: "Inactive",
        avatarUrl: null,
        provider: "gmail_api",
        isActive: false,
      },
      {
        id: "imap-active",
        email: "user@example.com",
        displayName: "IMAP",
        avatarUrl: null,
        provider: "imap",
        isActive: true,
      },
    ]);

    const { initializeClients } = await loadTokenManager();
    await initializeClients();

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("gmail_init_client", {
      accountId: "gmail-active",
    });
  });

  it("continues initializing later accounts when one init fails", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    mockListAccountBasicInfo.mockResolvedValue([
      {
        id: "gmail-1",
        email: "one@gmail.com",
        displayName: "One",
        avatarUrl: null,
        provider: "gmail_api",
        isActive: true,
      },
      {
        id: "gmail-2",
        email: "two@gmail.com",
        displayName: "Two",
        avatarUrl: null,
        provider: "gmail_api",
        isActive: true,
      },
    ]);
    mockInvoke
      .mockRejectedValueOnce(new Error("boom"))
      .mockResolvedValueOnce(undefined);

    const { initializeClients } = await loadTokenManager();
    await initializeClients();

    expect(mockInvoke).toHaveBeenNthCalledWith(1, "gmail_init_client", {
      accountId: "gmail-1",
    });
    expect(mockInvoke).toHaveBeenNthCalledWith(2, "gmail_init_client", {
      accountId: "gmail-2",
    });
    expect(errorSpy).toHaveBeenCalled();
  });
});
