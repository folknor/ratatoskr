import { vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  completeTask,
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
  uncompleteTask,
  updateTask,
  upsertTaskTag,
} from "./tasks";

beforeEach(() => {
  vi.clearAllMocks();
});

describe("tasks DB service", () => {
  describe("getTasksForAccount", () => {
    it("invokes db_get_tasks_for_account without completed", async () => {
      vi.mocked(invoke).mockResolvedValue([]);
      await getTasksForAccount("acc1");
      expect(invoke).toHaveBeenCalledWith("db_get_tasks_for_account", {
        accountId: "acc1",
        includeCompleted: false,
      });
    });

    it("includes completed tasks when requested", async () => {
      vi.mocked(invoke).mockResolvedValue([]);
      await getTasksForAccount("acc1", true);
      expect(invoke).toHaveBeenCalledWith("db_get_tasks_for_account", {
        accountId: "acc1",
        includeCompleted: true,
      });
    });
  });

  describe("getTaskById", () => {
    it("returns task when found", async () => {
      const task = { id: "t1", title: "Test" };
      vi.mocked(invoke).mockResolvedValue(task);
      const result = await getTaskById("t1");
      expect(result).toEqual(task);
    });

    it("returns null when not found", async () => {
      vi.mocked(invoke).mockResolvedValue(null);
      const result = await getTaskById("nonexistent");
      expect(result).toBeNull();
    });
  });

  describe("getTasksForThread", () => {
    it("invokes db_get_tasks_for_thread", async () => {
      vi.mocked(invoke).mockResolvedValue([]);
      await getTasksForThread("acc1", "thread1");
      expect(invoke).toHaveBeenCalledWith("db_get_tasks_for_thread", {
        accountId: "acc1",
        threadId: "thread1",
      });
    });
  });

  describe("getSubtasks", () => {
    it("invokes db_get_subtasks", async () => {
      vi.mocked(invoke).mockResolvedValue([]);
      await getSubtasks("parent1");
      expect(invoke).toHaveBeenCalledWith("db_get_subtasks", {
        parentId: "parent1",
      });
    });
  });

  describe("insertTask", () => {
    it("inserts a task with defaults", async () => {
      const id = await insertTask({ accountId: "acc1", title: "Buy milk" });
      expect(id).toBeTruthy();
      expect(invoke).toHaveBeenCalledWith(
        "db_insert_task",
        expect.objectContaining({
          accountId: "acc1",
          title: "Buy milk",
        }),
      );
    });

    it("uses provided id if given", async () => {
      const id = await insertTask({
        id: "custom-id",
        accountId: "acc1",
        title: "Test",
      });
      expect(id).toBe("custom-id");
    });
  });

  describe("updateTask", () => {
    it("updates specified fields", async () => {
      await updateTask("t1", { title: "Updated", priority: "high" });
      expect(invoke).toHaveBeenCalledWith(
        "db_update_task",
        expect.objectContaining({
          id: "t1",
          title: "Updated",
          priority: "high",
        }),
      );
    });
  });

  describe("deleteTask", () => {
    it("deletes by id", async () => {
      await deleteTask("t1");
      expect(invoke).toHaveBeenCalledWith("db_delete_task", { id: "t1" });
    });
  });

  describe("completeTask", () => {
    it("invokes db_complete_task", async () => {
      await completeTask("t1");
      expect(invoke).toHaveBeenCalledWith("db_complete_task", { id: "t1" });
    });
  });

  describe("uncompleteTask", () => {
    it("invokes db_uncomplete_task", async () => {
      await uncompleteTask("t1");
      expect(invoke).toHaveBeenCalledWith("db_uncomplete_task", { id: "t1" });
    });
  });

  describe("reorderTasks", () => {
    it("invokes db_reorder_tasks with all task IDs", async () => {
      await reorderTasks(["t1", "t2", "t3"]);
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke).toHaveBeenCalledWith("db_reorder_tasks", {
        taskIds: ["t1", "t2", "t3"],
      });
    });
  });

  describe("getIncompleteTaskCount", () => {
    it("returns count", async () => {
      vi.mocked(invoke).mockResolvedValue(5);
      const result = await getIncompleteTaskCount("acc1");
      expect(result).toBe(5);
    });
  });

  describe("task tags", () => {
    it("getTaskTags invokes correctly", async () => {
      vi.mocked(invoke).mockResolvedValue([]);
      await getTaskTags("acc1");
      expect(invoke).toHaveBeenCalledWith("db_get_task_tags", {
        accountId: "acc1",
      });
    });

    it("upsertTaskTag inserts with color", async () => {
      await upsertTaskTag("urgent", "acc1", "#ff0000");
      expect(invoke).toHaveBeenCalledWith("db_upsert_task_tag", {
        tag: "urgent",
        accountId: "acc1",
        color: "#ff0000",
      });
    });

    it("deleteTaskTag removes tag", async () => {
      await deleteTaskTag("urgent", "acc1");
      expect(invoke).toHaveBeenCalledWith("db_delete_task_tag", {
        tag: "urgent",
        accountId: "acc1",
      });
    });
  });
});
