import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";

interface SyncStateState {
  isOnline: boolean;
  pendingOpsCount: number;
  isSyncingFolder: string | null;
  setOnline: (online: boolean) => void;
  setPendingOpsCount: (count: number) => void;
  setSyncingFolder: (folder: string | null) => void;
}

export const useSyncStateStore: UseBoundStore<StoreApi<SyncStateState>> =
  create<SyncStateState>((set) => ({
    isOnline: true,
    pendingOpsCount: 0,
    isSyncingFolder: null,

    setOnline: (isOnline: boolean) => set({ isOnline }),
    setPendingOpsCount: (pendingOpsCount: number) => set({ pendingOpsCount }),
    setSyncingFolder: (isSyncingFolder: string | null) =>
      set({ isSyncingFolder }),
  }));
