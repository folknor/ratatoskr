/**
 * Core tasks facade — re-exports every task-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

// Task DB operations
export {
  completeTask,
  type DbTask,
  type DbTaskTag,
  deleteTask,
  deleteTaskTag,
  getIncompleteTaskCount,
  getSubtasks,
  getTaskById,
  getTasksForAccount,
  getTasksForThread,
  getTaskTags,
  insertTask,
  reorderTasks,
  type TaskPriority,
  uncompleteTask,
  updateTask,
  upsertTaskTag,
} from "@/services/db/tasks";

// Task manager (recurring tasks)
export { handleRecurringTaskCompletion } from "@/services/tasks/taskManager";
