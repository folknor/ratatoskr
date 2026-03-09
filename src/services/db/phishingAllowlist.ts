import { getDb } from "./connection";
import { normalizeEmail } from "@/utils/emailUtils";

export async function isPhishingAllowlisted(
  accountId: string,
  senderAddress: string,
): Promise<boolean> {
  const db = await getDb();
  const rows = await db.select<{ id: string }[]>(
    "SELECT id FROM phishing_allowlist WHERE account_id = $1 AND sender_address = $2 LIMIT 1",
    [accountId, normalizeEmail(senderAddress)],
  );
  return rows.length > 0;
}

export async function addToPhishingAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  const db = await getDb();
  const id = crypto.randomUUID();
  await db.execute(
    "INSERT OR IGNORE INTO phishing_allowlist (id, account_id, sender_address) VALUES ($1, $2, $3)",
    [id, accountId, normalizeEmail(senderAddress)],
  );
}

export async function removeFromPhishingAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  const db = await getDb();
  await db.execute(
    "DELETE FROM phishing_allowlist WHERE account_id = $1 AND sender_address = $2",
    [accountId, normalizeEmail(senderAddress)],
  );
}

export async function getPhishingAllowlist(
  accountId: string,
): Promise<{ id: string; sender_address: string; created_at: number }[]> {
  const db = await getDb();
  return db.select<{ id: string; sender_address: string; created_at: number }[]>(
    "SELECT id, sender_address, created_at FROM phishing_allowlist WHERE account_id = $1 ORDER BY sender_address",
    [accountId],
  );
}
