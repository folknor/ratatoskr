import { setSetting } from "@/core/settings";

/** Fire-and-forget setting persistence with error logging. */
export function persistSetting(key: string, value: string): void {
  setSetting(key, value).catch((err: unknown) => {
    console.error(`Failed to persist setting "${key}":`, err);
  });
}
