import type React from "react";
import { useState } from "react";
import { snoozeThread } from "@/core/mutations";
import { useContextMenuStore } from "@/stores/contextMenuStore";
import { SnoozeDialog } from "../../email/SnoozeDialog";
import { MessageContextMenu } from "./MessageContextMenu";
import { SidebarLabelContextMenu } from "./SidebarLabelContextMenu";
import { SidebarNavContextMenu } from "./SidebarNavContextMenu";
import { ThreadContextMenu } from "./ThreadContextMenu";

export function ContextMenuPortal(): React.ReactNode {
  const menuType = useContextMenuStore((s) => s.menuType);
  const position = useContextMenuStore((s) => s.position);
  const data = useContextMenuStore((s) => s.data);
  const closeMenu = useContextMenuStore((s) => s.closeMenu);
  const [snoozeTarget, setSnoozeTarget] = useState<{
    threadIds: string[];
    accountId: string;
  } | null>(null);

  if (!menuType) {
    if (snoozeTarget) {
      return (
        <SnoozeDialog
          onSnooze={async (until: number) => {
            for (const id of snoozeTarget.threadIds) {
              await snoozeThread(snoozeTarget.accountId, id, [], until);
            }
            setSnoozeTarget(null);
          }}
          onClose={() => setSnoozeTarget(null)}
        />
      );
    }
    return null;
  }

  return (
    <>
      {menuType === "sidebarLabel" && (
        <SidebarLabelContextMenu
          position={position}
          data={data}
          onClose={closeMenu}
        />
      )}
      {menuType === "sidebarNav" && (
        <SidebarNavContextMenu
          position={position}
          data={data}
          onClose={closeMenu}
        />
      )}
      {menuType === "thread" && (
        <ThreadContextMenu
          position={position}
          data={data}
          onClose={closeMenu}
          onSnooze={setSnoozeTarget}
        />
      )}
      {menuType === "message" && (
        <MessageContextMenu
          position={position}
          data={data}
          onClose={closeMenu}
        />
      )}
      {snoozeTarget != null && (
        <SnoozeDialog
          onSnooze={async (until: number) => {
            for (const id of snoozeTarget.threadIds) {
              await snoozeThread(snoozeTarget.accountId, id, [], until);
            }
            setSnoozeTarget(null);
          }}
          onClose={() => setSnoozeTarget(null)}
        />
      )}
    </>
  );
}
