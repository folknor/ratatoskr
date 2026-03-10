import { invoke } from "@tauri-apps/api/core";
import type { FilterCriteria } from "./filters";

export interface DbSmartLabelRule {
  id: string;
  account_id: string;
  label_id: string;
  ai_description: string;
  criteria_json: string | null;
  is_enabled: boolean;
  sort_order: number;
  created_at: number;
}

export async function getSmartLabelRulesForAccount(
  accountId: string,
): Promise<DbSmartLabelRule[]> {
  return invoke<DbSmartLabelRule[]>("db_get_smart_label_rules_for_account", {
    accountId,
  });
}

export async function getEnabledSmartLabelRules(
  accountId: string,
): Promise<DbSmartLabelRule[]> {
  const all = await getSmartLabelRulesForAccount(accountId);
  return all.filter((r) => r.is_enabled);
}

export async function insertSmartLabelRule(rule: {
  accountId: string;
  labelId: string;
  aiDescription: string;
  criteria?: FilterCriteria | undefined;
  isEnabled?: boolean;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_smart_label_rule", {
    id,
    accountId: rule.accountId,
    labelId: rule.labelId,
    aiDescription: rule.aiDescription,
    criteriaJson: rule.criteria ? JSON.stringify(rule.criteria) : null,
    isEnabled: rule.isEnabled ?? true,
  });
  return id;
}

export async function updateSmartLabelRule(
  id: string,
  updates: {
    labelId?: string;
    aiDescription?: string;
    criteria?: FilterCriteria | null;
    isEnabled?: boolean;
  },
): Promise<void> {
  await invoke("db_update_smart_label_rule", {
    id,
    labelId: updates.labelId,
    aiDescription: updates.aiDescription,
    criteriaJson:
      updates.criteria !== undefined
        ? updates.criteria
          ? JSON.stringify(updates.criteria)
          : null
        : undefined,
    isEnabled: updates.isEnabled,
  });
}

export async function deleteSmartLabelRule(id: string): Promise<void> {
  await invoke("db_delete_smart_label_rule", { id });
}
