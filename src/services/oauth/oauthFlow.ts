import { invoke } from "@tauri-apps/api/core";
import type { OAuthProviderConfig } from "./providers";

export interface TokenResponse {
  access_token: string;
  refresh_token?: string;
  expires_in: number;
  token_type: string;
  scope?: string;
  id_token?: string;
}

/**
 * Refresh an expired access token for a non-Gmail provider.
 */
export async function refreshProviderToken(
  provider: OAuthProviderConfig,
  refreshToken: string,
  clientId: string,
  clientSecret?: string,
): Promise<TokenResponse> {
  // Use Rust backend for token refresh to avoid CORS issues
  return invoke<TokenResponse>("oauth_refresh_token", {
    tokenUrl: provider.tokenUrl,
    refreshToken,
    clientId,
    clientSecret: clientSecret || null,
    scope:
      provider.id === "microsoft" || provider.id === "microsoft_graph"
        ? provider.scopes.join(" ")
        : null,
  });
}
