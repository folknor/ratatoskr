import { invoke } from "@tauri-apps/api/core";

export interface DbTemplate {
  id: string;
  account_id: string | null;
  name: string;
  subject: string | null;
  body_html: string;
  shortcut: string | null;
  sort_order: number;
  created_at: number;
}

/**
 * Get all templates for an account (includes global templates where account_id IS NULL).
 */
export async function getTemplatesForAccount(
  accountId: string,
): Promise<DbTemplate[]> {
  return invoke<DbTemplate[]>("db_get_templates_for_account", {
    accountId,
  });
}

export async function insertTemplate(tmpl: {
  accountId: string | null;
  name: string;
  subject: string | null;
  bodyHtml: string;
  shortcut: string | null;
}): Promise<string> {
  return invoke<string>("db_insert_template", {
    accountId: tmpl.accountId,
    name: tmpl.name,
    subject: tmpl.subject,
    bodyHtml: tmpl.bodyHtml,
    shortcut: tmpl.shortcut,
  });
}

export async function updateTemplate(
  id: string,
  updates: {
    name?: string;
    subject?: string | null;
    bodyHtml?: string;
    shortcut?: string | null;
  },
): Promise<void> {
  await invoke("db_update_template", {
    id,
    name: updates.name ?? null,
    subject: updates.subject ?? null,
    subjectSet: updates.subject !== undefined,
    bodyHtml: updates.bodyHtml ?? null,
    shortcut: updates.shortcut ?? null,
    shortcutSet: updates.shortcut !== undefined,
  });
}

export async function deleteTemplate(id: string): Promise<void> {
  await invoke("db_delete_template", { id });
}
