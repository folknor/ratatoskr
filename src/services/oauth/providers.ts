export interface OAuthProviderConfig {
  id: string;
  name: string;
  authUrl: string;
  tokenUrl: string;
  scopes: string[];
  userInfoUrl?: string | undefined;
  /** Whether PKCE is required (Microsoft requires it, Yahoo supports it) */
  usePkce: boolean;
}

const providers: Record<string, OAuthProviderConfig> = {
  microsoft: {
    id: "microsoft",
    name: "Microsoft",
    authUrl:
      "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
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
  },
  microsoft_graph: {
    id: "microsoft_graph",
    name: "Microsoft (Graph)",
    authUrl: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
    tokenUrl: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
    scopes: [
      "Mail.ReadWrite",
      "Mail.Send",
      "MailboxSettings.ReadWrite",
      "offline_access",
      "openid",
      "profile",
      "email",
    ],
    userInfoUrl: undefined,
    usePkce: true,
  },
  yahoo: {
    id: "yahoo",
    name: "Yahoo",
    authUrl: "https://api.login.yahoo.com/oauth2/request_auth",
    tokenUrl: "https://api.login.yahoo.com/oauth2/get_token",
    scopes: ["mail-r", "mail-w", "openid", "sdps-r"],
    userInfoUrl: "https://api.login.yahoo.com/openid/v1/userinfo",
    usePkce: true,
  },
};

export function getOAuthProvider(id: string): OAuthProviderConfig | null {
  return providers[id] ?? null;
}

export function getAllOAuthProviders(): OAuthProviderConfig[] {
  return Object.values(providers);
}
