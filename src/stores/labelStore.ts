import { invoke } from "@tauri-apps/api/core";
import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import {
  deleteLabel as dbDeleteLabel,
  getLabelsForAccount,
  updateLabelSortOrder,
  upsertLabel,
} from "@/core/labels";
import type { ProviderFolderResult } from "@/services/email/types";

export interface Label {
  id: string;
  accountId: string;
  name: string;
  type: string;
  colorBg: string | null;
  colorFg: string | null;
  sortOrder: number;
}

// System labels that are already shown as nav items in the sidebar
const SYSTEM_LABEL_IDS: Set<string> = new Set([
  "INBOX",
  "SENT",
  "DRAFT",
  "TRASH",
  "SPAM",
  "STARRED",
  "UNREAD",
  "IMPORTANT",
  "SNOOZED",
  "CHAT",
]);

const CATEGORY_PREFIX = "CATEGORY_";

export function isSystemLabel(id: string): boolean {
  return SYSTEM_LABEL_IDS.has(id) || id.startsWith(CATEGORY_PREFIX);
}

interface LabelState {
  labels: Label[];
  isLoading: boolean;
  loadLabels: (accountId: string) => Promise<void>;
  clearLabels: () => void;
  createLabel: (
    accountId: string,
    name: string,
    color?: { textColor: string; backgroundColor: string },
  ) => Promise<void>;
  updateLabel: (
    accountId: string,
    labelId: string,
    updates: {
      name?: string;
      color?: { textColor: string; backgroundColor: string } | null;
    },
  ) => Promise<void>;
  deleteLabel: (accountId: string, labelId: string) => Promise<void>;
  reorderLabels: (accountId: string, labelIds: string[]) => Promise<void>;
}

export const useLabelStore: UseBoundStore<StoreApi<LabelState>> =
  create<LabelState>((set, get) => ({
    labels: [],
    isLoading: false,

    loadLabels: async (accountId: string) => {
      set({ isLoading: true });
      try {
        const dbLabels = await getLabelsForAccount(accountId);
        const labels: Label[] = dbLabels
          .filter((l) => !isSystemLabel(l.id))
          .map((l) => ({
            id: l.id,
            accountId: l.account_id,
            name: l.name,
            type: l.type,
            colorBg: l.color_bg,
            colorFg: l.color_fg,
            sortOrder: l.sort_order,
          }));
        set({ labels, isLoading: false });
      } catch (err) {
        console.error("Failed to load labels:", err);
        set({ isLoading: false });
      }
    },

    clearLabels: () => set({ labels: [], isLoading: false }),

    createLabel: async (
      accountId: string,
      name: string,
      color?: { textColor: string; backgroundColor: string },
    ) => {
      const folder = await invoke<ProviderFolderResult>(
        "provider_create_folder",
        {
          accountId,
          name,
          textColor: color?.textColor,
          bgColor: color?.backgroundColor,
        },
      );
      await upsertLabel({
        id: folder.id,
        accountId,
        name: folder.name,
        type: folder.folderType,
        colorBg: folder.colorBg ?? null,
        colorFg: folder.colorFg ?? null,
      });
      await get().loadLabels(accountId);
    },

    updateLabel: async (
      accountId: string,
      labelId: string,
      updates: {
        name?: string;
        color?: { textColor: string; backgroundColor: string } | null;
      },
    ) => {
      const existing = get().labels.find((label) => label.id === labelId);
      const newName = updates.name ?? existing?.name;
      if (!newName) {
        throw new Error("Cannot rename folder without a name.");
      }
      const folder = await invoke<ProviderFolderResult>(
        "provider_rename_folder",
        {
          accountId,
          folderId: labelId,
          newName,
          textColor: updates.color?.textColor,
          bgColor: updates.color?.backgroundColor,
        },
      );
      await upsertLabel({
        id: folder.id,
        accountId,
        name: folder.name,
        type: folder.folderType,
        colorBg: folder.colorBg ?? null,
        colorFg: folder.colorFg ?? null,
      });
      await get().loadLabels(accountId);
    },

    deleteLabel: async (accountId: string, labelId: string) => {
      await invoke("provider_delete_folder", { accountId, folderId: labelId });
      await dbDeleteLabel(accountId, labelId);
      await get().loadLabels(accountId);
    },

    reorderLabels: async (accountId: string, labelIds: string[]) => {
      const labelOrders = labelIds.map((id, index) => ({
        id,
        sortOrder: index,
      }));
      await updateLabelSortOrder(accountId, labelOrders);
      await get().loadLabels(accountId);
    },
  }));
