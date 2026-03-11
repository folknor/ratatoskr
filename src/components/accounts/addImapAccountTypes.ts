import type { SecurityType } from "@/core/accounts";

export type Step = "basic" | "imap" | "smtp" | "test";
export type AuthMode = "password" | "oauth2";

export interface FormState {
  email: string;
  displayName: string;
  imapUsername: string;
  imapHost: string;
  imapPort: number;
  imapSecurity: SecurityType;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SecurityType;
  password: string;
  smtpPassword: string;
  samePassword: boolean;
  acceptInvalidCerts: boolean;
  // OAuth2 fields
  authMode: AuthMode;
  oauthProvider: string | null;
  oauthClientId: string;
  oauthClientSecret: string;
  oauthAccessToken: string | null;
  oauthRefreshToken: string | null;
  oauthExpiresAt: number | null;
  oauthEmail: string | null;
  oauthPicture: string | null;
}

export const initialFormState: FormState = {
  email: "",
  displayName: "",
  imapUsername: "",
  imapHost: "",
  imapPort: 993,
  imapSecurity: "ssl",
  smtpHost: "",
  smtpPort: 465,
  smtpSecurity: "ssl",
  password: "",
  smtpPassword: "",
  samePassword: true,
  acceptInvalidCerts: false,
  authMode: "password",
  oauthProvider: null,
  oauthClientId: "",
  oauthClientSecret: "",
  oauthAccessToken: null,
  oauthRefreshToken: null,
  oauthExpiresAt: null,
  oauthEmail: null,
  oauthPicture: null,
};

export interface TestStatus {
  state: "idle" | "testing" | "success" | "error";
  message?: string;
}

export const inputClass =
  "w-full px-3 py-2 bg-bg-secondary border border-border-primary rounded-lg text-sm text-text-primary outline-none focus:border-accent transition-colors";
export const labelClass = "block text-xs font-medium text-text-secondary mb-1";
export const selectClass =
  "w-full px-3 py-2 bg-bg-secondary border border-border-primary rounded-lg text-sm text-text-primary outline-none focus:border-accent transition-colors appearance-none";
