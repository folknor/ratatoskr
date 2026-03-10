import { invoke } from "@tauri-apps/api/core";
import { normalizeEmail } from "@/utils/emailUtils";

export interface DbContact {
  id: string;
  email: string;
  display_name: string | null;
  avatar_url: string | null;
  frequency: number;
  last_contacted_at: number | null;
  notes: string | null;
}

export interface ContactAttachment {
  filename: string;
  mime_type: string | null;
  size: number | null;
  date: number;
}

export interface SameDomainContact {
  email: string;
  display_name: string | null;
  avatar_url: string | null;
}

/**
 * Search contacts by email or name prefix for autocomplete.
 */
export async function searchContacts(
  query: string,
  limit: number = 10,
): Promise<DbContact[]> {
  return invoke("db_search_contacts", { query, limit });
}

/**
 * Get all contacts, ordered by frequency descending.
 */
export async function getAllContacts(
  limit: number = 500,
  offset: number = 0,
): Promise<DbContact[]> {
  return invoke("db_get_all_contacts", { limit, offset });
}

/**
 * Update a contact's display name.
 */
export async function updateContact(
  id: string,
  displayName: string | null,
): Promise<void> {
  return invoke("db_update_contact", { id, displayName });
}

/**
 * Delete a contact by ID.
 */
export async function deleteContact(id: string): Promise<void> {
  return invoke("db_delete_contact", { id });
}

/**
 * Upsert a contact — bumps frequency if already exists.
 */
export async function upsertContact(
  email: string,
  displayName: string | null,
): Promise<void> {
  const id = crypto.randomUUID();
  return invoke("db_upsert_contact", {
    id,
    email: normalizeEmail(email),
    displayName,
  });
}

export async function getContactByEmail(
  email: string,
): Promise<DbContact | null> {
  return invoke("db_get_contact_by_email", { email: normalizeEmail(email) });
}

export interface ContactStats {
  emailCount: number;
  firstEmail: number | null;
  lastEmail: number | null;
}

export async function getContactStats(email: string): Promise<ContactStats> {
  return invoke("db_get_contact_stats", { email: normalizeEmail(email) });
}

export async function getRecentThreadsWithContact(
  email: string,
  limit: number = 5,
): Promise<
  {
    thread_id: string;
    subject: string | null;
    last_message_at: number | null;
  }[]
> {
  return invoke("db_get_recent_threads_with_contact", {
    email: normalizeEmail(email),
    limit,
  });
}

export async function updateContactAvatar(
  email: string,
  avatarUrl: string,
): Promise<void> {
  return invoke("db_update_contact_avatar", {
    email: normalizeEmail(email),
    avatarUrl,
  });
}

/**
 * Update a contact's notes by email.
 */
export async function updateContactNotes(
  email: string,
  notes: string | null,
): Promise<void> {
  return invoke("db_update_contact_notes", {
    email: normalizeEmail(email),
    notes: notes || null,
  });
}

/**
 * Get recent non-inline attachments from a contact.
 */
export async function getAttachmentsFromContact(
  email: string,
  limit: number = 5,
): Promise<ContactAttachment[]> {
  return invoke("db_get_attachments_from_contact", {
    email: normalizeEmail(email),
    limit,
  });
}

const PUBLIC_DOMAINS: Set<string> = new Set([
  "gmail.com",
  "googlemail.com",
  "outlook.com",
  "hotmail.com",
  "live.com",
  "yahoo.com",
  "yahoo.co.uk",
  "aol.com",
  "icloud.com",
  "me.com",
  "mac.com",
  "protonmail.com",
  "proton.me",
  "mail.com",
  "zoho.com",
  "yandex.com",
  "gmx.com",
  "gmx.net",
]);

/**
 * Get other contacts from the same email domain (e.g., colleagues).
 * Skips public email providers.
 */
export async function getContactsFromSameDomain(
  email: string,
  limit: number = 5,
): Promise<SameDomainContact[]> {
  const normalized = normalizeEmail(email);
  const atIdx = normalized.indexOf("@");
  if (atIdx === -1) return [];

  const domain = normalized.slice(atIdx + 1);
  if (PUBLIC_DOMAINS.has(domain)) return [];

  return invoke("db_get_contacts_from_same_domain", {
    email: normalized,
    limit,
  });
}

/**
 * Get the most recent auth_results JSON string for messages from this sender.
 */
export async function getLatestAuthResult(
  email: string,
): Promise<string | null> {
  return invoke("db_get_latest_auth_result", {
    email: normalizeEmail(email),
  });
}
