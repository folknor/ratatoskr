import { invoke } from "@tauri-apps/api/core";

export type SecurityType = "ssl" | "starttls" | "none";
export type AuthMethod = "password" | "oauth2";

export interface ServerSettings {
  imapHost: string;
  imapPort: number;
  imapSecurity: SecurityType;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SecurityType;
}

export interface WellKnownProviderResult {
  settings: ServerSettings;
  authMethods: AuthMethod[];
  oauthProviderId?: string | undefined;
  acceptInvalidCerts?: boolean | undefined;
}

// --- Rust discovery types ---

interface DiscoveredServerConfig {
  hostname: string;
  port: number;
  security: "tls" | "startTls" | "none";
  username: { type: string; value?: string };
}

interface DiscoveredAuthConfig {
  method: DiscoveredAuthMethod;
  alternatives: DiscoveredAuthMethod[];
}

type DiscoveredAuthMethod =
  | { type: "password" }
  | {
      type: "oauth2";
      providerId: string;
      authUrl: string;
      tokenUrl: string;
      scopes: string[];
      usePkce: boolean;
    }
  | { type: "oauth2Unsupported"; providerDomain: string };

interface DiscoveredProtocolOption {
  protocol:
    | { type: "gmailApi" }
    | { type: "microsoftGraph" }
    | { type: "jmap"; sessionUrl: string }
    | {
        type: "imap";
        incoming: DiscoveredServerConfig;
        outgoing: DiscoveredServerConfig;
      };
  auth: DiscoveredAuthConfig;
  providerName: string | null;
  source: { type: string };
}

interface DiscoveredConfig {
  email: string;
  domain: string;
  options: DiscoveredProtocolOption[];
  resolvedDomain: string | null;
}

function mapSecurity(sec: "tls" | "startTls" | "none"): SecurityType {
  if (sec === "tls") return "ssl";
  if (sec === "startTls") return "starttls";
  return "none";
}

function mapAuthMethods(auth: DiscoveredAuthConfig): AuthMethod[] {
  const methods: AuthMethod[] = [];
  for (const m of [auth.method, ...auth.alternatives]) {
    if (m.type === "password") {
      methods.push("password");
    } else if (m.type === "oauth2") {
      methods.push("oauth2");
    }
  }
  return methods;
}

function extractOAuthProviderId(
  auth: DiscoveredAuthConfig,
): string | undefined {
  for (const m of [auth.method, ...auth.alternatives]) {
    if (m.type === "oauth2") {
      return m.providerId;
    }
  }
  return;
}

/**
 * Given an email address, attempt to discover server settings via Rust backend.
 * Returns null if the email address is invalid or no IMAP option was found.
 */
export async function discoverSettings(
  email: string,
): Promise<WellKnownProviderResult | null> {
  try {
    const config = await invoke<DiscoveredConfig>("discover_email_config", {
      email,
    });

    // Find the first IMAP option
    const imapOption = config.options.find(
      (opt) => opt.protocol.type === "imap",
    );
    if (!imapOption || imapOption.protocol.type !== "imap") {
      // No IMAP option — might be Gmail API or Graph only.
      // Fall back to guessed settings for the form.
      const domain = extractDomain(email);
      if (!domain) return null;
      return {
        settings: guessServerSettings(domain),
        authMethods: ["password"],
      };
    }

    const { incoming, outgoing } = imapOption.protocol;

    return {
      settings: {
        imapHost: incoming.hostname,
        imapPort: incoming.port,
        imapSecurity: mapSecurity(incoming.security),
        smtpHost: outgoing.hostname,
        smtpPort: outgoing.port,
        smtpSecurity: mapSecurity(outgoing.security),
      },
      authMethods: mapAuthMethods(imapOption.auth),
      oauthProviderId: extractOAuthProviderId(imapOption.auth),
    };
  } catch {
    // Rust command failed — fall back to guessed settings
    const domain = extractDomain(email);
    if (!domain) return null;
    return {
      settings: guessServerSettings(domain),
      authMethods: ["password"],
    };
  }
}

/**
 * Extract the domain part from an email address.
 * Returns null if the email is invalid.
 */
export function extractDomain(email: string): string | null {
  const trimmed = email.trim().toLowerCase();
  const atIndex = trimmed.lastIndexOf("@");
  if (atIndex < 1 || atIndex === trimmed.length - 1) return null;
  return trimmed.slice(atIndex + 1);
}

/**
 * Generate default server settings based on the domain using common patterns.
 */
export function guessServerSettings(domain: string): ServerSettings {
  return {
    imapHost: `imap.${domain}`,
    imapPort: 993,
    imapSecurity: "ssl",
    smtpHost: `smtp.${domain}`,
    smtpPort: 587,
    smtpSecurity: "starttls",
  };
}

/**
 * Get the default SMTP port for a given security type.
 */
export function getDefaultSmtpPort(security: SecurityType): number {
  // biome-ignore lint/nursery/noUnnecessaryConditions: exhaustive switch on union type
  switch (security) {
    case "ssl":
      return 465;
    case "starttls":
      return 587;
    case "none":
      return 25;
  }
}

/**
 * Get the default IMAP port for a given security type.
 */
export function getDefaultImapPort(security: SecurityType): number {
  // biome-ignore lint/nursery/noUnnecessaryConditions: exhaustive switch on union type
  switch (security) {
    case "ssl":
      return 993;
    case "starttls":
      return 143;
    case "none":
      return 143;
  }
}
