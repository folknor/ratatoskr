import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import { setSetting } from "@/core/settings";

export interface Account {
  id: string;
  email: string;
  displayName: string | null;
  avatarUrl: string | null;
  isActive: boolean;
  provider?: string;
}

interface AccountState {
  accounts: Account[];
  activeAccountId: string | null;
  setAccounts: (accounts: Account[], restoredId?: string | null) => void;
  setActiveAccount: (id: string) => void;
  addAccount: (account: Account) => void;
  removeAccount: (id: string) => void;
}

export const useAccountStore: UseBoundStore<StoreApi<AccountState>> =
  create<AccountState>((set) => ({
    accounts: [],
    activeAccountId: null,

    setAccounts: (accounts: Account[], restoredId?: string | null) => {
      const activeId =
        restoredId && accounts.some((a) => a.id === restoredId)
          ? restoredId
          : (accounts[0]?.id ?? null);
      set({ accounts, activeAccountId: activeId });
    },

    setActiveAccount: (activeAccountId: string) => {
      setSetting("active_account_id", activeAccountId).catch(() => {});
      set({ activeAccountId });
    },

    addAccount: (account: Account) =>
      set((state) => ({
        accounts: [...state.accounts, account],
        activeAccountId: state.activeAccountId ?? account.id,
      })),

    removeAccount: (id: string) =>
      set((state) => {
        const accounts = state.accounts.filter((a) => a.id !== id);
        return {
          accounts,
          activeAccountId:
            state.activeAccountId === id
              ? (accounts[0]?.id ?? null)
              : state.activeAccountId,
        };
      }),
  }));
