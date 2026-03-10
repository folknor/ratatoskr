import { Pencil, Trash2 } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type DbSmartFolder,
  deleteSmartFolder,
  getSmartFolders,
  insertSmartFolder,
  updateSmartFolder,
} from "@/core/queries";
import { useAccountStore } from "@/stores/accountStore";
import { useSmartFolderStore } from "@/stores/smartFolderStore";

export function SmartFolderEditor(): React.ReactNode {
  const { t } = useTranslation("settings");
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const reloadStore = useSmartFolderStore((s) => s.loadFolders);
  const [folders, setFolders] = useState<DbSmartFolder[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);

  // Form state
  const [name, setName] = useState("");
  const [query, setQuery] = useState("");
  const [icon, setIcon] = useState("Search");
  const [color, setColor] = useState("");

  const loadFolders = useCallback(async (): Promise<void> => {
    const f = await getSmartFolders(activeAccountId ?? undefined);
    setFolders(f);
  }, [activeAccountId]);

  useEffect(() => {
    void loadFolders();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- loadFolders is stable, only re-run on activeAccountId change
  }, [loadFolders]);

  const resetForm = useCallback((): void => {
    setName("");
    setQuery("");
    setIcon("Search");
    setColor("");
    setEditingId(null);
    setShowForm(false);
  }, []);

  const handleSave = useCallback(async (): Promise<void> => {
    if (!(name.trim() && query.trim())) return;

    if (editingId) {
      await updateSmartFolder(editingId, {
        name: name.trim(),
        query: query.trim(),
        icon: icon.trim() || "Search",
        color: color.trim() || undefined,
      });
    } else {
      await insertSmartFolder({
        name: name.trim(),
        query: query.trim(),
        accountId: activeAccountId ?? undefined,
        icon: icon.trim() || "Search",
        color: color.trim() || undefined,
      });
    }

    resetForm();
    await loadFolders();
    await reloadStore(activeAccountId ?? undefined);
  }, [
    activeAccountId,
    name,
    query,
    icon,
    color,
    editingId,
    resetForm,
    loadFolders,
    reloadStore,
  ]);

  const handleEdit = useCallback((folder: DbSmartFolder): void => {
    setEditingId(folder.id);
    setName(folder.name);
    setQuery(folder.query);
    setIcon(folder.icon);
    setColor(folder.color ?? "");
    setShowForm(true);
  }, []);

  const handleDelete = useCallback(
    async (id: string): Promise<void> => {
      await deleteSmartFolder(id);
      if (editingId === id) resetForm();
      await loadFolders();
      await reloadStore(activeAccountId ?? undefined);
    },
    [editingId, resetForm, loadFolders, reloadStore, activeAccountId],
  );

  return (
    <div className="space-y-3">
      {folders.map((folder) => (
        <div
          key={folder.id}
          className="flex items-center justify-between py-2 px-3 bg-bg-secondary rounded-md"
        >
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-text-primary flex items-center gap-2">
              {folder.name}
              {folder.is_default && (
                <span className="text-[0.625rem] bg-accent/15 text-accent px-1.5 py-0.5 rounded">
                  {t("smartFolderEditor.default")}
                </span>
              )}
            </div>
            <div className="text-xs text-text-tertiary truncate">
              {folder.query}
            </div>
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={(): void => handleEdit(folder)}
              className="p-1 text-text-tertiary hover:text-text-primary"
              title={t("smartFolderEditor.edit")}
            >
              <Pencil size={13} />
            </button>
            {!folder.is_default && (
              <button
                type="button"
                onClick={(): void => void handleDelete(folder.id)}
                className="p-1 text-text-tertiary hover:text-danger"
                title={t("smartFolderEditor.delete")}
              >
                <Trash2 size={13} />
              </button>
            )}
          </div>
        </div>
      ))}

      {showForm ? (
        <div className="border border-border-primary rounded-md p-3 space-y-3">
          <input
            type="text"
            value={name}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setName(e.target.value)
            }
            placeholder={t("smartFolderEditor.folderName")}
            className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
          />
          <input
            type="text"
            value={query}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setQuery(e.target.value)
            }
            placeholder={t("smartFolderEditor.searchQuery")}
            className="w-full px-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
          />
          <div className="flex gap-3">
            <div className="flex-1">
              {/* biome-ignore lint/a11y/noLabelWithoutControl: label text is descriptive, input follows immediately */}
              <label className="text-xs text-text-secondary block mb-1">
                {t("smartFolderEditor.iconName")}
              </label>
              <input
                type="text"
                value={icon}
                onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                  setIcon(e.target.value)
                }
                placeholder={t("smartFolderEditor.iconSearch")}
                className="w-full px-3 py-1 bg-bg-tertiary border border-border-primary rounded text-xs text-text-primary outline-none focus:border-accent"
              />
              <p className="text-[0.625rem] text-text-tertiary mt-0.5">
                {t("smartFolderEditor.iconHelp")}
              </p>
            </div>
            <div className="flex-1">
              {/* biome-ignore lint/a11y/noLabelWithoutControl: label text is descriptive, input follows immediately */}
              <label className="text-xs text-text-secondary block mb-1">
                {t("smartFolderEditor.colorOptional")}
              </label>
              <input
                type="text"
                value={color}
                onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                  setColor(e.target.value)
                }
                placeholder={t("smartFolderEditor.colorPlaceholder")}
                className="w-full px-3 py-1 bg-bg-tertiary border border-border-primary rounded text-xs text-text-primary outline-none focus:border-accent"
              />
            </div>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={(): void => void handleSave()}
              disabled={!(name.trim() && query.trim())}
              className="px-3 py-1.5 text-xs font-medium text-white bg-accent hover:bg-accent-hover rounded-md transition-colors disabled:opacity-50"
            >
              {editingId
                ? t("smartFolderEditor.update")
                : t("smartFolderEditor.save")}
            </button>
            <button
              type="button"
              onClick={resetForm}
              className="px-3 py-1.5 text-xs text-text-secondary hover:text-text-primary rounded-md transition-colors"
            >
              {t("smartFolderEditor.cancel")}
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={(): void => setShowForm(true)}
          className="text-xs text-accent hover:text-accent-hover"
        >
          {t("smartFolderEditor.addSmartFolder")}
        </button>
      )}
    </div>
  );
}
