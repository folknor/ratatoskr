import { Download, ExternalLink, Eye } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import type { AttachmentWithContext } from "@/services/db/attachments";
import {
  canPreview,
  formatFileSize,
  getFileIcon,
} from "@/utils/fileTypeHelpers";

interface AttachmentGridItemProps {
  attachment: AttachmentWithContext;
  onPreview: () => void;
  onDownload: () => void;
  onJumpToEmail: () => void;
}

function formatRelativeDate(timestamp: number | null): string {
  if (!timestamp) return "";
  const diff = Date.now() - timestamp;
  const mins = Math.floor(diff / 60000);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(months / 12)}y ago`;
}

export function AttachmentGridItem({
  attachment,
  onPreview,
  onDownload,
  onJumpToEmail,
}: AttachmentGridItemProps): React.ReactNode {
  const { t } = useTranslation("attachments");
  const previewable = canPreview(attachment.mime_type, attachment.filename);
  const senderName =
    attachment.from_name || attachment.from_address || t("unknown");

  return (
    <div className="group relative flex flex-col border border-border-primary rounded-lg hover:border-border-secondary hover:bg-bg-hover transition-colors overflow-hidden">
      {/* Icon area */}
      <button
        type="button"
        onClick={previewable ? onPreview : onDownload}
        className="flex items-center justify-center h-24 bg-bg-secondary text-3xl"
      >
        {getFileIcon(attachment.mime_type)}
      </button>

      {/* Info */}
      <div className="px-3 py-2 flex flex-col gap-0.5 min-w-0">
        <span
          className="text-xs font-medium text-text-primary truncate"
          title={attachment.filename ?? undefined}
        >
          {attachment.filename ?? t("unnamed")}
        </span>
        <span
          className="text-[0.6875rem] text-text-tertiary truncate"
          title={senderName}
        >
          {senderName}
        </span>
        <div className="flex items-center gap-2 text-[0.6875rem] text-text-tertiary">
          {attachment.size != null && (
            <span>{formatFileSize(attachment.size)}</span>
          )}
          {attachment.date != null && (
            <span>{formatRelativeDate(attachment.date)}</span>
          )}
        </div>
      </div>

      {/* Hover actions */}
      <div className="absolute top-1.5 right-1.5 flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
        {previewable && (
          <button
            type="button"
            onClick={onPreview}
            className="p-1.5 rounded-md bg-bg-primary/90 border border-border-primary text-text-secondary hover:text-text-primary transition-colors"
            title={t("preview")}
          >
            <Eye size={13} />
          </button>
        )}
        <button
          type="button"
          onClick={onDownload}
          className="p-1.5 rounded-md bg-bg-primary/90 border border-border-primary text-text-secondary hover:text-text-primary transition-colors"
          title={t("download")}
        >
          <Download size={13} />
        </button>
        <button
          type="button"
          onClick={onJumpToEmail}
          className="p-1.5 rounded-md bg-bg-primary/90 border border-border-primary text-text-secondary hover:text-text-primary transition-colors"
          title={t("jumpToEmail")}
        >
          <ExternalLink size={13} />
        </button>
      </div>
    </div>
  );
}
