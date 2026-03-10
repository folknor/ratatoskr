import { Pencil, RefreshCw, Trash2 } from "lucide-react";
import type React from "react";
import { triggerSync } from "@/core/mutations";
import { useAccountStore } from "@/stores/accountStore";
import { useSyncStateStore } from "@/stores/syncStateStore";
import { ContextMenu, type ContextMenuItem } from "../ContextMenu";
import type { MenuComponentProps } from "./types";

export function SidebarLabelContextMenu({
  position,
  data,
  onClose,
}: MenuComponentProps): React.ReactNode {
  const onEdit = data["onEdit"] as (() => void) | undefined;
  const onDelete = data["onDelete"] as (() => void) | undefined;
  const activeAccountId = useAccountStore((s) => s.activeAccountId);

  const handleSync = (): void => {
    if (!activeAccountId) return;
    const labelId = data["labelId"] as string | undefined;
    useSyncStateStore.getState().setSyncingFolder(labelId ?? "label");
    triggerSync([activeAccountId]);
  };

  const items: ContextMenuItem[] = [
    {
      id: "sync-folder",
      label: "Sync this folder",
      icon: RefreshCw,
      action: handleSync,
    },
    { id: "sep-sync", label: "", separator: true },
    {
      id: "edit-label",
      label: "Edit label",
      icon: Pencil,
      action: () => onEdit?.(),
    },
    {
      id: "delete-label",
      label: "Delete label",
      icon: Trash2,
      danger: true,
      action: () => onDelete?.(),
    },
  ];

  return <ContextMenu items={items} position={position} onClose={onClose} />;
}
