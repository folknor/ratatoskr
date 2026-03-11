import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OAuthProviderConfig } from "@/services/oauth/providers";

// Mock Tauri APIs
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { refreshProviderToken } from "@/services/oauth/oauthFlow";

const microsoftProvider: OAuthProviderConfig = {
  id: "microsoft",
  name: "Microsoft",
  authUrl: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
  tokenUrl: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
  scopes: [
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "https://outlook.office.com/SMTP.Send",
    "offline_access",
    "openid",
    "profile",
    "email",
  ],
  userInfoUrl: undefined,
  usePkce: true,
};

const yahooProvider: OAuthProviderConfig = {
  id: "yahoo",
  name: "Yahoo",
  authUrl: "https://api.login.yahoo.com/oauth2/request_auth",
  tokenUrl: "https://api.login.yahoo.com/oauth2/get_token",
  scopes: ["mail-r", "mail-w", "openid"],
  userInfoUrl: "https://api.login.yahoo.com/openid/v1/userinfo",
  usePkce: true,
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("refreshProviderToken", () => {
  it("invokes Rust oauth_refresh_token for Microsoft with scope", async () => {
    vi.mocked(invoke).mockResolvedValue({
      access_token: "new-access",
      refresh_token: "new-refresh",
      expires_in: 3600,
      token_type: "Bearer",
    });

    const result = await refreshProviderToken(
      microsoftProvider,
      "old-refresh",
      "client-123",
    );

    expect(invoke).toHaveBeenCalledWith("oauth_refresh_token", {
      tokenUrl: microsoftProvider.tokenUrl,
      refreshToken: "old-refresh",
      clientId: "client-123",
      clientSecret: null,
      scope: microsoftProvider.scopes.join(" "),
    });
    expect(result.access_token).toBe("new-access");
  });

  it("invokes Rust oauth_refresh_token for Yahoo without scope", async () => {
    vi.mocked(invoke).mockResolvedValue({
      access_token: "yahoo-token",
      expires_in: 3600,
      token_type: "Bearer",
    });

    await refreshProviderToken(yahooProvider, "yahoo-refresh", "yahoo-client");

    expect(invoke).toHaveBeenCalledWith("oauth_refresh_token", {
      tokenUrl: yahooProvider.tokenUrl,
      refreshToken: "yahoo-refresh",
      clientId: "yahoo-client",
      clientSecret: null,
      scope: null,
    });
  });

  it("passes clientSecret when provided", async () => {
    vi.mocked(invoke).mockResolvedValue({
      access_token: "token",
      expires_in: 3600,
      token_type: "Bearer",
    });

    await refreshProviderToken(
      yahooProvider,
      "refresh",
      "client",
      "secret-123",
    );

    expect(invoke).toHaveBeenCalledWith("oauth_refresh_token", {
      tokenUrl: yahooProvider.tokenUrl,
      refreshToken: "refresh",
      clientId: "client",
      clientSecret: "secret-123",
      scope: null,
    });
  });

  it("propagates errors from invoke", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("Token refresh failed: 400"));

    await expect(
      refreshProviderToken(microsoftProvider, "bad-refresh", "client"),
    ).rejects.toThrow("Token refresh failed: 400");
  });
});
