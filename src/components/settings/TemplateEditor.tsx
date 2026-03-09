import Image from "@tiptap/extension-image";
import Placeholder from "@tiptap/extension-placeholder";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { ChevronDown, Pencil, Trash2 } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { EditorToolbar } from "@/components/composer/EditorToolbar";
import {
  type DbTemplate,
  deleteTemplate,
  getTemplatesForAccount,
  insertTemplate,
  updateTemplate,
} from "@/core/composer";
import { useAccountStore } from "@/stores/accountStore";
import { TEMPLATE_VARIABLES } from "@/utils/templateVariables";

export function TemplateEditor(): React.ReactNode {
  const { t } = useTranslation("settings");
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const [templates, setTemplates] = useState<DbTemplate[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [subject, setSubject] = useState("");
  const [shortcut, setShortcut] = useState("");
  const [showForm, setShowForm] = useState(false);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: { levels: [1, 2, 3] },
        link: { openOnClick: false },
      }),
      Image.configure({ inline: true, allowBase64: true }),
      Placeholder.configure({ placeholder: "Write your template..." }),
    ],
    content: "",
    editorProps: {
      attributes: {
        class:
          "prose prose-sm max-w-none px-3 py-2 min-h-[80px] focus:outline-none text-text-primary text-xs",
      },
    },
  });

  const loadTemplates = useCallback(async (): Promise<void> => {
    if (!activeAccountId) return;
    const tmpls = await getTemplatesForAccount(activeAccountId);
    setTemplates(tmpls);
  }, [activeAccountId]);

  useEffect(() => {
    void loadTemplates();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- loadTemplates is stable, only re-run on activeAccountId change
  }, [loadTemplates]);

  const resetForm = useCallback((): void => {
    setName("");
    setSubject("");
    setShortcut("");
    setEditingId(null);
    setShowForm(false);
    editor?.commands.setContent("");
  }, [editor]);

  const handleSave = useCallback(async (): Promise<void> => {
    if (!(activeAccountId && editor && name.trim())) return;

    const bodyHtml = editor.getHTML();

    if (editingId) {
      await updateTemplate(editingId, {
        name: name.trim(),
        subject: subject.trim() || null,
        bodyHtml,
        shortcut: shortcut.trim() || null,
      });
    } else {
      await insertTemplate({
        accountId: activeAccountId,
        name: name.trim(),
        subject: subject.trim() || null,
        bodyHtml,
        shortcut: shortcut.trim() || null,
      });
    }

    resetForm();
    await loadTemplates();
  }, [
    activeAccountId,
    editor,
    name,
    subject,
    shortcut,
    editingId,
    resetForm,
    loadTemplates,
  ]);

  const handleEdit = useCallback(
    (tmpl: DbTemplate): void => {
      setEditingId(tmpl.id);
      setName(tmpl.name);
      setSubject(tmpl.subject ?? "");
      setShortcut(tmpl.shortcut ?? "");
      setShowForm(true);
      editor?.commands.setContent(tmpl.body_html);
    },
    [editor],
  );

  const handleDelete = useCallback(
    async (id: string): Promise<void> => {
      await deleteTemplate(id);
      if (editingId === id) resetForm();
      await loadTemplates();
    },
    [editingId, resetForm, loadTemplates],
  );

  return (
    <div className="space-y-3">
      {templates.map((tmpl) => (
        <div
          key={tmpl.id}
          className="flex items-center justify-between py-2 px-3 bg-bg-secondary rounded-md"
        >
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-text-primary flex items-center gap-2">
              {tmpl.name}
              {tmpl.shortcut != null && (
                <kbd className="text-[0.625rem] bg-bg-tertiary text-text-tertiary px-1.5 py-0.5 rounded">
                  {tmpl.shortcut}
                </kbd>
              )}
            </div>
            {tmpl.subject != null && (
              <div className="text-xs text-text-tertiary truncate">
                {tmpl.subject}
              </div>
            )}
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={(): void => handleEdit(tmpl)}
              className="p-1 text-text-tertiary hover:text-text-primary"
            >
              <Pencil size={13} />
            </button>
            <button
              type="button"
              onClick={(): void => void handleDelete(tmpl.id)}
              className="p-1 text-text-tertiary hover:text-danger"
            >
              <Trash2 size={13} />
            </button>
          </div>
        </div>
      ))}

      {showForm ? (
        <div className="border border-border-primary rounded-md p-3 space-y-2">
          <input
            type="text"
            value={name}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setName(e.target.value)
            }
            placeholder={t("templateEditor.templateName")}
            className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
          />
          <input
            type="text"
            value={subject}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setSubject(e.target.value)
            }
            placeholder={t("templateEditor.subjectOptional")}
            className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
          />
          <div className="border border-border-primary rounded overflow-hidden bg-bg-tertiary">
            <EditorToolbar editor={editor} />
            <EditorContent editor={editor} />
          </div>
          <InsertVariableDropdown
            onInsert={(variable: string): void => {
              editor?.chain().focus().insertContent(variable).run();
            }}
          />
          <input
            type="text"
            value={shortcut}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setShortcut(e.target.value)
            }
            placeholder={t("templateEditor.shortcutOptional")}
            className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
          />
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={(): void => void handleSave()}
              disabled={!name.trim()}
              className="px-3 py-1.5 text-xs font-medium text-white bg-accent hover:bg-accent-hover rounded-md transition-colors disabled:opacity-50"
            >
              {editingId
                ? t("templateEditor.update")
                : t("templateEditor.save")}
            </button>
            <button
              type="button"
              onClick={resetForm}
              className="px-3 py-1.5 text-xs text-text-secondary hover:text-text-primary rounded-md transition-colors"
            >
              {t("templateEditor.cancel")}
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={(): void => setShowForm(true)}
          className="text-xs text-accent hover:text-accent-hover"
        >
          {t("templateEditor.addTemplate")}
        </button>
      )}
    </div>
  );
}

function InsertVariableDropdown({
  onInsert,
}: {
  onInsert: (variable: string) => void;
}): React.ReactNode {
  const { t } = useTranslation("settings");
  const [open, setOpen] = useState(false);

  return (
    <div className="relative">
      <button
        type="button"
        onClick={(): void => setOpen(!open)}
        className="flex items-center gap-1 text-xs text-accent hover:text-accent-hover transition-colors"
      >
        {t("templateEditor.insertVariable")}
        <ChevronDown
          size={12}
          className={
            open ? "rotate-180 transition-transform" : "transition-transform"
          }
        />
      </button>
      {open === true && (
        <div className="absolute left-0 top-full mt-1 z-10 bg-bg-primary border border-border-primary rounded-md shadow-lg py-1 min-w-[220px]">
          {TEMPLATE_VARIABLES.map((v) => (
            <button
              key={v.key}
              type="button"
              onClick={(): void => {
                onInsert(v.key);
                setOpen(false);
              }}
              className="w-full text-left px-3 py-1.5 hover:bg-bg-hover text-xs flex items-center justify-between gap-3"
            >
              <code className="text-accent">{v.key}</code>
              <span className="text-text-tertiary">{v.desc}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
