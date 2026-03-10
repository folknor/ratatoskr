import { invoke } from "@tauri-apps/api/core";

export interface FilterCriteria {
  from?: string;
  to?: string;
  subject?: string;
  body?: string;
  hasAttachment?: boolean;
}

export interface FilterActions {
  applyLabel?: string;
  archive?: boolean;
  star?: boolean;
  markRead?: boolean;
  trash?: boolean;
}

export interface DbFilterRule {
  id: string;
  account_id: string;
  name: string;
  is_enabled: boolean;
  criteria_json: string;
  actions_json: string;
  sort_order: number;
  created_at: number;
}

export async function getFiltersForAccount(
  accountId: string,
): Promise<DbFilterRule[]> {
  return invoke<DbFilterRule[]>("db_get_filters_for_account", { accountId });
}

export async function getEnabledFiltersForAccount(
  accountId: string,
): Promise<DbFilterRule[]> {
  const all = await getFiltersForAccount(accountId);
  return all.filter((f) => f.is_enabled);
}

export async function insertFilter(filter: {
  accountId: string;
  name: string;
  criteria: FilterCriteria;
  actions: FilterActions;
  isEnabled?: boolean;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_filter", {
    id,
    accountId: filter.accountId,
    name: filter.name,
    criteriaJson: JSON.stringify(filter.criteria),
    actionsJson: JSON.stringify(filter.actions),
    isEnabled: filter.isEnabled ?? true,
  });
  return id;
}

export async function updateFilter(
  id: string,
  updates: {
    name?: string;
    criteria?: FilterCriteria;
    actions?: FilterActions;
    isEnabled?: boolean;
  },
): Promise<void> {
  await invoke("db_update_filter", {
    id,
    name: updates.name,
    criteriaJson: updates.criteria
      ? JSON.stringify(updates.criteria)
      : undefined,
    actionsJson: updates.actions ? JSON.stringify(updates.actions) : undefined,
    isEnabled: updates.isEnabled,
  });
}

export async function deleteFilter(id: string): Promise<void> {
  await invoke("db_delete_filter", { id });
}
