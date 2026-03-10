import { RefreshCw } from "lucide-react";
import type React from "react";
import { triggerSync } from "@/core/mutations";
import { useAccountStore } from "@/stores/accountStore";
import { useSyncStateStore } from "@/stores/syncStateStore";
import { ContextMenu, type ContextMenuItem } from "../ContextMenu";
import type { MenuComponentProps } from "./types";

export function SidebarNavContextMenu({
  position,
  data,
  onClose,
}: MenuComponentProps): React.ReactNode {
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const navId = data["navId"] as string;

  const handleSync = (): void => {
    if (!activeAccountId) return;
    useSyncStateStore.getState().setSyncingFolder(navId);
    triggerSync([activeAccountId]);
  };

  const items: ContextMenuItem[] = [
    {
      id: "sync-folder",
      label: "Sync this folder",
      icon: RefreshCw,
      action: handleSync,
    },
  ];

  return <ContextMenu items={items} position={position} onClose={onClose} />;
}
