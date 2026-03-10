import { invoke } from "@tauri-apps/api/core";
import type { QuickStepAction } from "../quickSteps/types";

export interface DbQuickStep {
  id: string;
  account_id: string;
  name: string;
  description: string | null;
  shortcut: string | null;
  actions_json: string;
  icon: string | null;
  is_enabled: boolean;
  continue_on_error: boolean;
  sort_order: number;
  created_at: number;
}

export async function getQuickStepsForAccount(
  accountId: string,
): Promise<DbQuickStep[]> {
  return invoke<DbQuickStep[]>("db_get_quick_steps_for_account", {
    accountId,
  });
}

export async function getEnabledQuickStepsForAccount(
  accountId: string,
): Promise<DbQuickStep[]> {
  return invoke<DbQuickStep[]>("db_get_enabled_quick_steps_for_account", {
    accountId,
  });
}

export async function insertQuickStep(step: {
  accountId: string;
  name: string;
  description?: string | undefined;
  shortcut?: string | undefined;
  actions: QuickStepAction[];
  icon?: string | undefined;
  isEnabled?: boolean | undefined;
  continueOnError?: boolean | undefined;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_quick_step", {
    step: {
      id,
      account_id: step.accountId,
      name: step.name,
      description: step.description ?? null,
      shortcut: step.shortcut ?? null,
      actions_json: JSON.stringify(step.actions),
      icon: step.icon ?? null,
      is_enabled: step.isEnabled !== false,
      continue_on_error: step.continueOnError ?? false,
      sort_order: 0,
      created_at: 0,
    },
  });
  return id;
}

export async function updateQuickStep(
  id: string,
  updates: {
    name?: string | undefined;
    description?: string | undefined;
    shortcut?: string | null | undefined;
    actions?: QuickStepAction[] | undefined;
    icon?: string | undefined;
    isEnabled?: boolean | undefined;
    continueOnError?: boolean | undefined;
  },
): Promise<void> {
  // Rust db_update_quick_step takes a full DbQuickStep struct.
  // Pass the updated fields; non-provided fields use safe defaults.
  await invoke("db_update_quick_step", {
    step: {
      id,
      account_id: "",
      name: updates.name ?? "",
      description: updates.description ?? null,
      shortcut: updates.shortcut ?? null,
      actions_json: updates.actions ? JSON.stringify(updates.actions) : "[]",
      icon: updates.icon ?? null,
      is_enabled: updates.isEnabled ?? true,
      continue_on_error: updates.continueOnError ?? false,
      sort_order: 0,
      created_at: 0,
    },
  });
}

export async function deleteQuickStep(id: string): Promise<void> {
  await invoke("db_delete_quick_step", { id });
}

export async function reorderQuickSteps(
  accountId: string,
  orderedIds: string[],
): Promise<void> {
  // Fetch all steps, update sort_order for each by position
  const steps = await getQuickStepsForAccount(accountId);
  const stepMap = new Map(steps.map((s) => [s.id, s]));

  for (let i = 0; i < orderedIds.length; i++) {
    const step = stepMap.get(orderedIds[i]!);
    if (step) {
      await invoke("db_update_quick_step", {
        step: { ...step, sort_order: i },
      });
    }
  }
}
