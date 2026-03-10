import { invoke } from "@tauri-apps/api/core";

export interface DbSendAsAlias {
  id: string;
  account_id: string;
  email: string;
  display_name: string | null;
  reply_to_address: string | null;
  signature_id: string | null;
  is_primary: number;
  is_default: number;
  treat_as_alias: number;
  verification_status: string;
  created_at: number;
}

export interface SendAsAlias {
  id: string;
  accountId: string;
  email: string;
  displayName: string | null;
  replyToAddress: string | null;
  signatureId: string | null;
  isPrimary: boolean;
  isDefault: boolean;
  treatAsAlias: boolean;
  verificationStatus: string;
}

export function mapDbAlias(db: DbSendAsAlias): SendAsAlias {
  return {
    id: db.id,
    accountId: db.account_id,
    email: db.email,
    displayName: db.display_name,
    replyToAddress: db.reply_to_address,
    signatureId: db.signature_id,
    isPrimary: db.is_primary === 1,
    isDefault: db.is_default === 1,
    treatAsAlias: db.treat_as_alias === 1,
    verificationStatus: db.verification_status,
  };
}

export async function getAliasesForAccount(
  accountId: string,
): Promise<DbSendAsAlias[]> {
  return invoke<DbSendAsAlias[]>("db_get_aliases_for_account", {
    accountId,
  });
}

export async function upsertAlias(alias: {
  accountId: string;
  email: string;
  displayName?: string | null;
  replyToAddress?: string | null;
  signatureId?: string | null;
  isPrimary?: boolean;
  isDefault?: boolean;
  treatAsAlias?: boolean;
  verificationStatus?: string;
}): Promise<string> {
  return invoke<string>("db_upsert_alias", {
    accountId: alias.accountId,
    email: alias.email,
    displayName: alias.displayName ?? null,
    replyToAddress: alias.replyToAddress ?? null,
    signatureId: alias.signatureId ?? null,
    isPrimary: alias.isPrimary ?? false,
    isDefault: alias.isDefault ?? false,
    treatAsAlias: alias.treatAsAlias !== false,
    verificationStatus: alias.verificationStatus ?? "accepted",
  });
}

export async function getDefaultAlias(
  accountId: string,
): Promise<DbSendAsAlias | null> {
  return invoke<DbSendAsAlias | null>("db_get_default_alias", {
    accountId,
  });
}

export async function setDefaultAlias(
  accountId: string,
  aliasId: string,
): Promise<void> {
  return invoke<void>("db_set_default_alias", { accountId, aliasId });
}

export async function deleteAlias(id: string): Promise<void> {
  return invoke<void>("db_delete_alias", { id });
}
