import { getDb, existsBy } from "./connection";
import { normalizeEmail } from "@/utils/emailUtils";

export interface NotificationVip {
  id: string;
  account_id: string;
  email_address: string;
  display_name: string | null;
  created_at: number;
}

export async function getVipSenders(accountId: string): Promise<Set<string>> {
  const db = await getDb();
  const rows = await db.select<{ email_address: string }[]>(
    "SELECT email_address FROM notification_vips WHERE account_id = $1",
    [accountId],
  );
  return new Set(rows.map((r) => normalizeEmail(r.email_address)));
}

export async function getAllVipSenders(accountId: string): Promise<NotificationVip[]> {
  const db = await getDb();
  return db.select<NotificationVip[]>(
    "SELECT * FROM notification_vips WHERE account_id = $1 ORDER BY display_name, email_address",
    [accountId],
  );
}

export async function addVipSender(
  accountId: string,
  email: string,
  displayName?: string,
): Promise<void> {
  const db = await getDb();
  const id = crypto.randomUUID();
  await db.execute(
    "INSERT OR IGNORE INTO notification_vips (id, account_id, email_address, display_name) VALUES ($1, $2, $3, $4)",
    [id, accountId, normalizeEmail(email), displayName ?? null],
  );
}

export async function removeVipSender(
  accountId: string,
  email: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "DELETE FROM notification_vips WHERE account_id = $1 AND email_address = $2",
    [accountId, normalizeEmail(email)],
  );
}

export async function isVipSender(
  accountId: string,
  email: string,
): Promise<boolean> {
  return existsBy(
    "SELECT COUNT(*) as count FROM notification_vips WHERE account_id = $1 AND email_address = $2",
    [accountId, normalizeEmail(email)],
  );
}
