import type { Editor } from "@tiptap/react";
import { useRef } from "react";

import type { DbTemplate } from "@/core/composer";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";
import { interpolateVariables } from "@/utils/templateVariables";

/**
 * Manages template shortcuts loaded from the DB.
 * Returns a ref to update loaded templates and a handler to check/apply
 * shortcut triggers inside the TipTap editor's onUpdate callback.
 */
export function useTemplateShortcuts(setSubject: (subject: string) => void): {
  templateShortcutsRef: React.RefObject<DbTemplate[]>;
  checkTemplateShortcut: (ed: Editor) => void;
} {
  const templateShortcutsRef = useRef<DbTemplate[]>([]);

  const checkTemplateShortcut = (ed: Editor): void => {
    const templates = templateShortcutsRef.current;
    if (templates.length === 0) return;

    const text = ed.state.doc.textContent;
    for (const tmpl of templates) {
      if (!tmpl.shortcut) continue;
      if (text.endsWith(tmpl.shortcut)) {
        // Delete the shortcut text and insert template body with variables resolved
        const { from } = ed.state.selection;
        const deleteFrom = from - tmpl.shortcut.length;
        if (deleteFrom >= 0) {
          const state = useComposerStore.getState();
          const account = useAccountStore
            .getState()
            .accounts.find(
              (a) => a.id === useAccountStore.getState().activeAccountId,
            );
          void interpolateVariables(tmpl.body_html, {
            recipientEmail: state.to[0],
            senderEmail: account?.email,
            senderName: account?.displayName ?? undefined,
            subject: state.subject || undefined,
          }).then((resolved) => {
            ed.chain()
              .deleteRange({ from: deleteFrom, to: from })
              .insertContent(resolved)
              .run();
          });
          if (tmpl.subject && !state.subject) {
            setSubject(tmpl.subject);
          }
        }
        break;
      }
    }
  };

  return { templateShortcutsRef, checkTemplateShortcut };
}
