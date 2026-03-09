import Image from "@tiptap/extension-image";
import Placeholder from "@tiptap/extension-placeholder";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Code, Pencil, Trash2 } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { EditorToolbar } from "@/components/composer/EditorToolbar";
import { TextField } from "@/components/ui/TextField";
import {
  type DbSignature,
  deleteSignature,
  getSignaturesForAccount,
  insertSignature,
  updateSignature,
} from "@/services/db/signatures";
import { useAccountStore } from "@/stores/accountStore";

export function SignatureEditor(): React.ReactNode {
  const { t } = useTranslation("settings");
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const [signatures, setSignatures] = useState<DbSignature[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [isDefault, setIsDefault] = useState(false);
  const [showForm, setShowForm] = useState(false);
  const [isHtmlMode, setIsHtmlMode] = useState(false);
  const [rawHtml, setRawHtml] = useState("");

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: { levels: [1, 2, 3] },
        link: { openOnClick: false },
      }),
      Image.configure({ inline: true, allowBase64: true }),
      Placeholder.configure({ placeholder: "Write your signature..." }),
    ],
    content: "",
    editorProps: {
      attributes: {
        class:
          "prose prose-sm max-w-none px-3 py-2 min-h-[80px] focus:outline-none text-text-primary text-xs",
      },
    },
  });

  const loadSignatures = useCallback(async (): Promise<void> => {
    if (!activeAccountId) return;
    const sigs = await getSignaturesForAccount(activeAccountId);
    setSignatures(sigs);
  }, [activeAccountId]);

  useEffect(() => {
    void loadSignatures();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- loadSignatures is stable, only re-run on activeAccountId change
  }, [loadSignatures]);

  const resetForm = useCallback((): void => {
    setName("");
    setIsDefault(false);
    setEditingId(null);
    setShowForm(false);
    setIsHtmlMode(false);
    setRawHtml("");
    editor?.commands.setContent("");
  }, [editor]);

  const toggleHtmlMode = useCallback((): void => {
    if (!editor) return;
    if (isHtmlMode) {
      // HTML → WYSIWYG: push rawHtml into editor
      editor.commands.setContent(rawHtml);
    } else {
      // WYSIWYG → HTML: capture editor content
      setRawHtml(editor.getHTML());
    }
    setIsHtmlMode(!isHtmlMode);
  }, [editor, isHtmlMode, rawHtml]);

  const handleSave = useCallback(async (): Promise<void> => {
    if (!(activeAccountId && editor && name.trim())) return;

    const bodyHtml = isHtmlMode ? rawHtml : editor.getHTML();

    if (editingId) {
      await updateSignature(editingId, {
        name: name.trim(),
        bodyHtml,
        isDefault,
      });
    } else {
      await insertSignature({
        accountId: activeAccountId,
        name: name.trim(),
        bodyHtml,
        isDefault,
      });
    }

    resetForm();
    await loadSignatures();
  }, [
    activeAccountId,
    editor,
    name,
    isDefault,
    editingId,
    isHtmlMode,
    rawHtml,
    resetForm,
    loadSignatures,
  ]);

  const handleEdit = useCallback(
    (sig: DbSignature): void => {
      setEditingId(sig.id);
      setName(sig.name);
      setIsDefault(sig.is_default === 1);
      setShowForm(true);
      editor?.commands.setContent(sig.body_html);
    },
    [editor],
  );

  const handleDelete = useCallback(
    async (id: string): Promise<void> => {
      await deleteSignature(id);
      if (editingId === id) resetForm();
      await loadSignatures();
    },
    [editingId, resetForm, loadSignatures],
  );

  return (
    <div className="space-y-3">
      {signatures.map((sig) => (
        <div
          key={sig.id}
          className="flex items-center justify-between py-2 px-3 bg-bg-secondary rounded-md"
        >
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-text-primary flex items-center gap-2">
              {sig.name}
              {sig.is_default === 1 && (
                <span className="text-[0.625rem] bg-accent/10 text-accent px-1.5 py-0.5 rounded">
                  {t("signatureEditor.default")}
                </span>
              )}
            </div>
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={(): void => handleEdit(sig)}
              className="p-1 text-text-tertiary hover:text-text-primary"
            >
              <Pencil size={13} />
            </button>
            <button
              type="button"
              onClick={(): void => void handleDelete(sig.id)}
              className="p-1 text-text-tertiary hover:text-danger"
            >
              <Trash2 size={13} />
            </button>
          </div>
        </div>
      ))}

      {showForm ? (
        <div className="border border-border-primary rounded-md p-3 space-y-2">
          <TextField
            type="text"
            value={name}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setName(e.target.value)
            }
            placeholder={t("signatureEditor.signatureName")}
          />
          <div className="border border-border-primary rounded overflow-hidden bg-bg-tertiary">
            <div className="flex items-center justify-between">
              {isHtmlMode ? (
                <span className="px-2 py-1 text-xs text-text-secondary">
                  {t("signatureEditor.htmlSource")}
                </span>
              ) : (
                <EditorToolbar editor={editor} />
              )}
              <button
                type="button"
                onClick={toggleHtmlMode}
                className={`p-1.5 mr-1 rounded transition-colors ${isHtmlMode ? "text-accent bg-accent/10" : "text-text-tertiary hover:text-text-primary"}`}
                title={
                  isHtmlMode
                    ? t("signatureEditor.switchToVisual")
                    : t("signatureEditor.editHtml")
                }
              >
                <Code size={14} />
              </button>
            </div>
            {isHtmlMode ? (
              <textarea
                value={rawHtml}
                onChange={(e: React.ChangeEvent<HTMLTextAreaElement>): void =>
                  setRawHtml(e.target.value)
                }
                className="w-full px-3 py-2 min-h-[80px] bg-bg-tertiary text-text-primary text-xs font-mono focus:outline-none resize-y"
                spellCheck={false}
              />
            ) : (
              <EditorContent editor={editor} />
            )}
          </div>
          <div className="flex items-center gap-2">
            <label className="flex items-center gap-1.5 text-xs text-text-secondary">
              <input
                type="checkbox"
                checked={isDefault}
                onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                  setIsDefault(e.target.checked)
                }
                className="rounded"
              />
              {t("signatureEditor.setAsDefault")}
            </label>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={(): void => void handleSave()}
              disabled={!name.trim()}
              className="px-3 py-1.5 text-xs font-medium text-white bg-accent hover:bg-accent-hover rounded-md transition-colors disabled:opacity-50"
            >
              {editingId
                ? t("signatureEditor.update")
                : t("signatureEditor.save")}
            </button>
            <button
              type="button"
              onClick={resetForm}
              className="px-3 py-1.5 text-xs text-text-secondary hover:text-text-primary rounded-md transition-colors"
            >
              {t("signatureEditor.cancel")}
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={(): void => setShowForm(true)}
          className="text-xs text-accent hover:text-accent-hover"
        >
          {t("signatureEditor.addSignature")}
        </button>
      )}
    </div>
  );
}
