import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import { getDefaultKeyMap } from "@/constants/shortcuts";
import { setSetting } from "@/core/settings";
import { getCustomShortcutOverrides } from "@/services/settings/runtimeFlags";

interface ShortcutState {
  /** Map of shortcut ID -> current key binding */
  keyMap: Record<string, string>;
  /** Load custom bindings from DB, merging with defaults */
  loadKeyMap: (rawOverrides?: string | null) => Promise<void>;
  /** Update a single shortcut binding */
  setKey: (id: string, keys: string) => void;
  /** Reset a single shortcut to its default */
  resetKey: (id: string) => void;
  /** Reset all shortcuts to defaults */
  resetAll: () => void;
}

const SETTINGS_KEY = "custom_shortcuts";

function persistKeyMap(customKeys: Record<string, string>): void {
  const defaults = getDefaultKeyMap();
  // Only persist non-default bindings
  const overrides: Record<string, string> = {};
  for (const [id, keys] of Object.entries(customKeys)) {
    if (defaults[id] !== keys) {
      overrides[id] = keys;
    }
  }
  setSetting(SETTINGS_KEY, JSON.stringify(overrides)).catch(() => {});
}

export const useShortcutStore: UseBoundStore<StoreApi<ShortcutState>> =
  create<ShortcutState>((set, get) => ({
    keyMap: getDefaultKeyMap(),

    loadKeyMap: async (rawOverrides: string | null = null) => {
      const defaults = getDefaultKeyMap();
      try {
        const raw = rawOverrides ?? (await getCustomShortcutOverrides());
        if (raw) {
          const overrides = JSON.parse(raw) as Record<string, string>;
          set({ keyMap: { ...defaults, ...overrides } });
        }
      } catch {
        // Use defaults on parse error
      }
    },

    setKey: (id: string, keys: string) => {
      const updated = { ...get().keyMap, [id]: keys };
      set({ keyMap: updated });
      persistKeyMap(updated);
    },

    resetKey: (id: string) => {
      const defaults = getDefaultKeyMap();
      const defaultKey = defaults[id];
      if (defaultKey) {
        const updated = { ...get().keyMap, [id]: defaultKey };
        set({ keyMap: updated });
        persistKeyMap(updated);
      }
    },

    resetAll: () => {
      const defaults = getDefaultKeyMap();
      set({ keyMap: defaults });
      setSetting(SETTINGS_KEY, "{}").catch(() => {});
    },
  }));
