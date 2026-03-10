import { invoke } from "@tauri-apps/api/core";
import { upsertAlias } from "../db/sendAsAliases";

interface GmailSendAs {
  sendAsEmail: string;
  displayName?: string;
  replyToAddress?: string;
  isPrimary?: boolean;
  treatAsAlias?: boolean;
  verificationStatus?: string;
}

/**
 * Fetch send-as aliases from the Rust backend and store them locally.
 */
export async function fetchSendAsAliases(accountId: string): Promise<void> {
  const aliases = await invoke<GmailSendAs[]>("gmail_fetch_send_as", {
    accountId,
  });

  for (const entry of aliases) {
    await upsertAlias({
      accountId,
      email: entry.sendAsEmail,
      displayName: entry.displayName ?? null,
      replyToAddress: entry.replyToAddress ?? null,
      isPrimary: entry.isPrimary ?? false,
      treatAsAlias: entry.treatAsAlias ?? true,
      verificationStatus: entry.verificationStatus ?? "accepted",
    });
  }
}
