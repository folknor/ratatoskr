import { invoke } from "@tauri-apps/api/core";

export type TaskPriority = "none" | "low" | "medium" | "high" | "urgent";

export interface DbTask {
  id: string;
  account_id: string | null;
  title: string;
  description: string | null;
  priority: TaskPriority;
  is_completed: number;
  completed_at: number | null;
  due_date: number | null;
  parent_id: string | null;
  thread_id: string | null;
  thread_account_id: string | null;
  sort_order: number;
  recurrence_rule: string | null;
  next_recurrence_at: number | null;
  tags_json: string;
  created_at: number;
  updated_at: number;
}

export interface DbTaskTag {
  tag: string;
  account_id: string | null;
  color: string | null;
  sort_order: number;
  created_at: number;
}

export async function getTasksForAccount(
  accountId: string | null,
  includeCompleted: boolean = false,
): Promise<DbTask[]> {
  return invoke<DbTask[]>("db_get_tasks_for_account", {
    accountId,
    includeCompleted,
  });
}

export async function getTaskById(id: string): Promise<DbTask | null> {
  return invoke<DbTask | null>("db_get_task_by_id", { id });
}

export async function getTasksForThread(
  accountId: string,
  threadId: string,
): Promise<DbTask[]> {
  return invoke<DbTask[]>("db_get_tasks_for_thread", {
    accountId,
    threadId,
  });
}

export async function getSubtasks(parentId: string): Promise<DbTask[]> {
  return invoke<DbTask[]>("db_get_subtasks", { parentId });
}

export async function insertTask(task: {
  id?: string;
  accountId: string | null;
  title: string;
  description?: string | null;
  priority?: TaskPriority;
  dueDate?: number | null;
  parentId?: string | null;
  threadId?: string | null;
  threadAccountId?: string | null;
  sortOrder?: number;
  recurrenceRule?: string | null;
  tagsJson?: string;
}): Promise<string> {
  const id = task.id ?? crypto.randomUUID();
  await invoke<void>("db_insert_task", {
    id,
    accountId: task.accountId,
    title: task.title,
    description: task.description ?? null,
    priority: task.priority ?? "none",
    dueDate: task.dueDate ?? null,
    parentId: task.parentId ?? null,
    threadId: task.threadId ?? null,
    threadAccountId: task.threadAccountId ?? null,
    sortOrder: task.sortOrder ?? 0,
    recurrenceRule: task.recurrenceRule ?? null,
    tagsJson: task.tagsJson ?? "[]",
  });
  return id;
}

export async function updateTask(
  id: string,
  updates: {
    title?: string;
    description?: string | null;
    priority?: TaskPriority;
    dueDate?: number | null;
    sortOrder?: number;
    recurrenceRule?: string | null;
    nextRecurrenceAt?: number | null;
    tagsJson?: string;
  },
): Promise<void> {
  return invoke<void>("db_update_task", {
    id,
    title: updates.title,
    description:
      updates.description !== undefined && updates.description !== null
        ? updates.description
        : undefined,
    priority: updates.priority,
    dueDate:
      updates.dueDate !== undefined && updates.dueDate !== null
        ? updates.dueDate
        : undefined,
    sortOrder: updates.sortOrder,
    recurrenceRule:
      updates.recurrenceRule !== undefined && updates.recurrenceRule !== null
        ? updates.recurrenceRule
        : undefined,
    nextRecurrenceAt:
      updates.nextRecurrenceAt !== undefined &&
      updates.nextRecurrenceAt !== null
        ? updates.nextRecurrenceAt
        : undefined,
    tagsJson: updates.tagsJson,
    // Sentinel flags: set to true when the caller explicitly passes null
    clearDescription:
      updates.description === null && updates.description !== undefined,
    clearDueDate: updates.dueDate === null && updates.dueDate !== undefined,
    clearRecurrenceRule:
      updates.recurrenceRule === null && updates.recurrenceRule !== undefined,
    clearNextRecurrenceAt:
      updates.nextRecurrenceAt === null &&
      updates.nextRecurrenceAt !== undefined,
  });
}

export async function deleteTask(id: string): Promise<void> {
  return invoke<void>("db_delete_task", { id });
}

export async function completeTask(id: string): Promise<void> {
  return invoke<void>("db_complete_task", { id });
}

export async function uncompleteTask(id: string): Promise<void> {
  return invoke<void>("db_uncomplete_task", { id });
}

export async function reorderTasks(taskIds: string[]): Promise<void> {
  return invoke<void>("db_reorder_tasks", { taskIds });
}

export async function getIncompleteTaskCount(
  accountId: string | null,
): Promise<number> {
  return invoke<number>("db_get_incomplete_task_count", { accountId });
}

export async function getTaskTags(
  accountId: string | null,
): Promise<DbTaskTag[]> {
  return invoke<DbTaskTag[]>("db_get_task_tags", { accountId });
}

export async function upsertTaskTag(
  tag: string,
  accountId: string | null,
  color?: string | null,
): Promise<void> {
  return invoke<void>("db_upsert_task_tag", {
    tag,
    accountId,
    color: color ?? null,
  });
}

export async function deleteTaskTag(
  tag: string,
  accountId: string | null,
): Promise<void> {
  return invoke<void>("db_delete_task_tag", { tag, accountId });
}
