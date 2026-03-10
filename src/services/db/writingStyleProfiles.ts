import { invoke } from "@tauri-apps/api/core";

export interface DbWritingStyleProfile {
  id: string;
  account_id: string;
  profile_text: string;
  sample_count: number;
  created_at: number;
  updated_at: number;
}

export async function getWritingStyleProfile(
  accountId: string,
): Promise<DbWritingStyleProfile | null> {
  return invoke<DbWritingStyleProfile | null>(
    "db_get_writing_style_profile",
    { accountId },
  );
}

export async function upsertWritingStyleProfile(
  accountId: string,
  profileText: string,
  sampleCount: number,
): Promise<void> {
  await invoke("db_upsert_writing_style_profile", {
    accountId,
    profileText,
    sampleCount,
  });
}

export async function deleteWritingStyleProfile(
  accountId: string,
): Promise<void> {
  await invoke("db_delete_writing_style_profile", { accountId });
}
