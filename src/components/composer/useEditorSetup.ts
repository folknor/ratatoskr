import Image from "@tiptap/extension-image";
import Placeholder from "@tiptap/extension-placeholder";
import type { Editor } from "@tiptap/react";
import { useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { useTranslation } from "react-i18next";

import { useComposerStore } from "@/stores/composerStore";

/**
 * Configures and returns a TipTap editor instance with all extensions,
 * editor props, and the onUpdate handler wired up.
 *
 * @param checkTemplateShortcut - callback invoked on every editor update to
 *   detect and apply template shortcut triggers.
 */
export function useEditorSetup(
  checkTemplateShortcut: (ed: Editor) => void,
): ReturnType<typeof useEditor> {
  const { t } = useTranslation("composer");

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: { levels: [1, 2, 3] },
        link: { openOnClick: false },
      }),
      Placeholder.configure({
        placeholder: t("writePlaceholder"),
      }),
      Image.configure({
        inline: true,
        allowBase64: true,
      }),
    ],
    content: useComposerStore.getState().bodyHtml,
    // biome-ignore lint/nursery/useExplicitType: tiptap callback
    onUpdate: ({ editor: ed }) => {
      useComposerStore.getState().setBodyHtml(ed.getHTML());

      // Check for template shortcut triggers
      checkTemplateShortcut(ed);
    },
    editorProps: {
      attributes: {
        class:
          "prose prose-sm max-w-none px-4 py-3 min-h-[200px] focus:outline-none text-text-primary",
      },
      // biome-ignore lint/nursery/useExplicitType: tiptap callback
      handleDrop: (_view, event) => {
        // Prevent TipTap from handling file drops as inline content.
        // Returning true stops TipTap's Image extension from intercepting the drop,
        // allowing the event to bubble up to the composer's onDrop for attachment handling.
        if (event.dataTransfer?.files?.length) {
          return true;
        }
        return false;
      },
    },
  });

  return editor;
}
