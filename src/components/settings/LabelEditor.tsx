import { ChevronDown, ChevronUp, Pencil, Trash2, X } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LabelForm } from "@/components/labels/LabelForm";
import { useAccountStore } from "@/stores/accountStore";
import { type Label, useLabelStore } from "@/stores/labelStore";

export function LabelEditor(): React.ReactNode {
  const { t } = useTranslation("settings");
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const { labels, loadLabels, deleteLabel, reorderLabels } = useLabelStore();

  const [editingId, setEditingId] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (activeAccountId) {
      void loadLabels(activeAccountId);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- loadLabels is a stable store function, only re-run on activeAccountId change
  }, [activeAccountId, loadLabels]);

  const resetForm = useCallback((): void => {
    setEditingId(null);
    setShowForm(false);
    setError(null);
  }, []);

  const handleEdit = useCallback((label: Label): void => {
    setEditingId(label.id);
    setShowForm(true);
    setError(null);
  }, []);

  const handleDelete = useCallback(
    async (label: Label): Promise<void> => {
      if (!activeAccountId) return;
      setError(null);
      try {
        await deleteLabel(activeAccountId, label.id);
        if (editingId === label.id) resetForm();
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to delete label");
      }
    },
    [activeAccountId, deleteLabel, editingId, resetForm],
  );

  const handleMoveUp = useCallback(
    async (index: number): Promise<void> => {
      if (!activeAccountId || index === 0) return;
      const newOrder = labels.map((l) => l.id);
      const prev = newOrder[index - 1];
      const curr = newOrder[index];
      if (prev != null && curr != null) {
        newOrder[index - 1] = curr;
        newOrder[index] = prev;
      }
      await reorderLabels(activeAccountId, newOrder);
    },
    [activeAccountId, labels, reorderLabels],
  );

  const handleMoveDown = useCallback(
    async (index: number): Promise<void> => {
      if (!activeAccountId || index >= labels.length - 1) return;
      const newOrder = labels.map((l) => l.id);
      const curr = newOrder[index];
      const next = newOrder[index + 1];
      if (curr != null && next != null) {
        newOrder[index] = next;
        newOrder[index + 1] = curr;
      }
      await reorderLabels(activeAccountId, newOrder);
    },
    [activeAccountId, labels, reorderLabels],
  );

  const editingLabel = editingId
    ? (labels.find((l) => l.id === editingId) ?? null)
    : null;

  return (
    <div className="space-y-3">
      {error != null && (
        <div className="flex items-center gap-2 px-3 py-2 bg-danger/10 text-danger text-xs rounded-md">
          <span className="flex-1">{error}</span>
          <button
            type="button"
            onClick={(): void => setError(null)}
            className="shrink-0"
          >
            <X size={12} />
          </button>
        </div>
      )}

      {labels.length === 0 && !showForm && (
        <p className="text-sm text-text-tertiary">
          {t("labelEditor.noUserLabels")}
        </p>
      )}

      {labels.map((label, index) => (
        <div key={label.id}>
          <div className="flex items-center justify-between py-2 px-3 bg-bg-secondary rounded-md">
            <div className="flex items-center gap-2 flex-1 min-w-0">
              {label.colorBg != null ? (
                <span
                  className="w-3 h-3 rounded-full shrink-0"
                  style={{ backgroundColor: label.colorBg }}
                />
              ) : (
                <span className="w-3 h-3 rounded-full shrink-0 bg-text-tertiary/30" />
              )}
              <span className="text-sm font-medium text-text-primary truncate">
                {label.name}
              </span>
            </div>
            <div className="flex items-center gap-0.5">
              <button
                type="button"
                onClick={(): void => void handleMoveUp(index)}
                disabled={index === 0}
                className="p-1 text-text-tertiary hover:text-text-primary disabled:opacity-30 disabled:cursor-not-allowed"
                title={t("labelEditor.moveUp")}
              >
                <ChevronUp size={13} />
              </button>
              <button
                type="button"
                onClick={(): void => void handleMoveDown(index)}
                disabled={index === labels.length - 1}
                className="p-1 text-text-tertiary hover:text-text-primary disabled:opacity-30 disabled:cursor-not-allowed"
                title={t("labelEditor.moveDown")}
              >
                <ChevronDown size={13} />
              </button>
              <button
                type="button"
                onClick={(): void => handleEdit(label)}
                className="p-1 text-text-tertiary hover:text-text-primary"
                title={t("labelEditor.edit")}
              >
                <Pencil size={13} />
              </button>
              <button
                type="button"
                onClick={(): void => void handleDelete(label)}
                className="p-1 text-text-tertiary hover:text-danger"
                title={t("labelEditor.delete")}
              >
                <Trash2 size={13} />
              </button>
            </div>
          </div>
          {/* Inline edit form under the label being edited */}
          {showForm && editingId === label.id && activeAccountId != null && (
            <div className="mt-1">
              <LabelForm
                accountId={activeAccountId}
                label={editingLabel}
                onDone={resetForm}
              />
            </div>
          )}
        </div>
      ))}

      {/* New label form at bottom */}
      {showForm && editingId == null && activeAccountId != null ? (
        <LabelForm accountId={activeAccountId} onDone={resetForm} />
      ) : (
        !showForm && (
          <button
            type="button"
            onClick={(): void => {
              setShowForm(true);
              setEditingId(null);
              setError(null);
            }}
            className="text-xs text-accent hover:text-accent-hover"
          >
            {t("labelEditor.addLabel")}
          </button>
        )
      )}
    </div>
  );
}
