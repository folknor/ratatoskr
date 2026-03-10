import { MailMinus } from "lucide-react";
import type React from "react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DbAttachment } from "@/core/attachments";
import type { DbMessage } from "@/core/queries";
import { formatFullDate } from "@/utils/date";
import { AttachmentList, getAttachmentsForMessage } from "./AttachmentList";
import { AuthBadge } from "./AuthBadge";
import { AuthWarningBanner } from "./AuthWarningBanner";
import { EmailRenderer } from "./EmailRenderer";
import { InlineAttachmentPreview } from "./InlineAttachmentPreview";

interface MessageItemProps {
  message: DbMessage;
  isLast: boolean;
  blockImages?: boolean | null;
  senderAllowlisted?: boolean;
  accountId?: string;
  threadId?: string;
  isSpam?: boolean;
  focused?: boolean;
  onContextMenu?: (e: React.MouseEvent) => void;
}

export const MessageItem = memo(function MessageItemInner({
  message,
  isLast,
  blockImages,
  senderAllowlisted,
  accountId,
  threadId,
  isSpam,
  focused,
  onContextMenu,
  ref,
}: MessageItemProps & {
  ref?: React.Ref<HTMLDivElement | null>;
}): React.ReactNode {
  const { t } = useTranslation("email");
  const [expanded, setExpanded] = useState(isLast);
  const [attachments, setAttachments] = useState<DbAttachment[]>([]);
  const [authBannerDismissed, setAuthBannerDismissed] = useState(false);
  const attachmentsLoadedRef = useRef(false);

  const loadAttachments = useCallback(async (): Promise<void> => {
    if (attachmentsLoadedRef.current) return;
    attachmentsLoadedRef.current = true;
    try {
      const atts = await getAttachmentsForMessage(
        message.account_id,
        message.id,
      );
      setAttachments(atts);
    } catch {
      // Non-critical — just show no attachments
    }
  }, [message.account_id, message.id]);

  // Load attachments for initially-expanded (last) message on mount
  useEffect(() => {
    if (isLast) {
      void loadAttachments();
    }
  }, [isLast, loadAttachments]);

  // Auto-expand when focused via keyboard navigation
  useEffect(() => {
    if (focused && !expanded) {
      setExpanded(true);
      void loadAttachments();
    }
  }, [focused, expanded, loadAttachments]);

  const handleToggle = (): void => {
    const willExpand = !expanded;
    setExpanded(willExpand);
    if (willExpand) {
      void loadAttachments();
    }
  };

  // Scan HTML body for cid: references — these images are already rendered inline
  const referencedCids = useMemo(() => {
    const cids = new Set<string>();
    if (!message.body_html) return cids;
    const regex = /\bcid:([^"'\s)]+)/gi;
    let m: RegExpExecArray | null = regex.exec(message.body_html);
    while (m !== null) {
      const cid = m[1];
      if (cid) cids.add(cid);
      m = regex.exec(message.body_html);
    }
    return cids;
  }, [message.body_html]);

  const fromDisplay =
    message.from_name ?? message.from_address ?? t("common:unknown");

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: context menu on container div
    <div
      ref={ref}
      className={`border-b border-border-secondary last:border-b-0 ${isSpam ? "bg-red-500/8 dark:bg-red-500/10" : ""} ${focused ? "ring-2 ring-inset ring-accent/50" : ""}`}
      onContextMenu={onContextMenu}
    >
      {/* Header — always visible, click to expand/collapse */}
      <button
        type="button"
        onClick={handleToggle}
        className="w-full text-left px-4 py-3 hover:bg-bg-hover transition-colors"
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 min-w-0">
            <div className="w-7 h-7 rounded-full bg-accent/20 text-accent flex items-center justify-center shrink-0 text-xs font-medium">
              {fromDisplay[0]?.toUpperCase()}
            </div>
            <div className="min-w-0">
              <span className="text-sm font-medium text-text-primary truncate flex items-center gap-1">
                {fromDisplay}
                <AuthBadge authResults={message.auth_results} />
              </span>
              {!expanded && (
                <span className="text-xs text-text-tertiary truncate block">
                  {message.snippet}
                </span>
              )}
            </div>
          </div>
          <span className="text-xs text-text-tertiary whitespace-nowrap shrink-0 ml-2">
            {formatFullDate(message.date)}
          </span>
        </div>
        {expanded === true && (
          <div className="mt-1 text-xs text-text-tertiary">
            {message.to_addresses != null && (
              <span>
                {t("to")} {message.to_addresses}
              </span>
            )}
          </div>
        )}
      </button>

      {/* Body — shown when expanded and image setting resolved */}
      {expanded === true && (
        <div className="px-4 pb-4">
          {!authBannerDismissed && (
            <AuthWarningBanner
              authResults={message.auth_results}
              senderAddress={message.from_address}
              // biome-ignore lint/nursery/useExplicitType: inline callback
              onDismiss={() => setAuthBannerDismissed(true)}
            />
          )}

          {message.list_unsubscribe != null && (
            <UnsubscribeLink
              header={message.list_unsubscribe}
              postHeader={message.list_unsubscribe_post}
              accountId={accountId ?? message.account_id}
              threadId={threadId ?? message.thread_id}
              fromAddress={message.from_address}
              fromName={message.from_name}
            />
          )}

          {blockImages != null ? (
            <EmailRenderer
              html={message.body_html}
              text={message.body_text}
              blockImages={blockImages}
              senderAddress={message.from_address}
              accountId={message.account_id}
              senderAllowlisted={senderAllowlisted}
              messageId={message.id}
              inlineAttachments={attachments.filter((a) => a.content_id)}
            />
          ) : (
            <div className="py-8 text-center text-text-tertiary text-sm">
              {t("loadingMessage")}
            </div>
          )}

          <InlineAttachmentPreview
            accountId={message.account_id}
            messageId={message.id}
            attachments={attachments}
            referencedCids={referencedCids}
            // biome-ignore lint/nursery/useExplicitType: inline noop callback
            onAttachmentClick={() => {}}
          />

          <AttachmentList
            accountId={message.account_id}
            messageId={message.id}
            attachments={attachments}
            referencedCids={referencedCids}
          />
        </div>
      )}
    </div>
  );
});

export function parseUnsubscribeUrl(header: string): string | null {
  // Prefer https URL over mailto
  const httpMatch = header.match(/<(https?:\/\/[^>]+)>/);
  if (httpMatch?.[1]) return httpMatch[1];
  const mailtoMatch = header.match(/<(mailto:[^>]+)>/);
  if (mailtoMatch?.[1]) return mailtoMatch[1];
  return null;
}

function UnsubscribeLink({
  header,
  postHeader,
  accountId,
  threadId,
  fromAddress,
  fromName,
}: {
  header: string;
  postHeader?: string | null;
  accountId: string;
  threadId: string;
  fromAddress: string | null;
  fromName: string | null;
}): React.ReactNode {
  const { t } = useTranslation("email");
  const url = parseUnsubscribeUrl(header);
  const [status, setStatus] = useState<"idle" | "loading" | "done" | "failed">(
    "idle",
  );
  if (!url) return null;

  const handleClick = async (e: React.MouseEvent): Promise<void> => {
    e.stopPropagation();
    setStatus("loading");
    try {
      const { executeUnsubscribe } = await import(
        "@/services/unsubscribe/unsubscribeManager"
      );
      const result = await executeUnsubscribe(
        accountId,
        threadId,
        fromAddress ?? "unknown",
        fromName,
        header,
        postHeader ?? null,
      );
      setStatus(result.success ? "done" : "failed");
    } catch (err) {
      console.error("Failed to unsubscribe:", err);
      setStatus("failed");
    }
  };

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={status === "loading" || status === "done"}
      className={`flex items-center gap-1 text-xs mb-2 transition-colors ${
        status === "done"
          ? "text-success"
          : status === "failed"
            ? "text-danger"
            : "text-text-tertiary hover:text-text-secondary"
      }`}
    >
      <MailMinus size={12} />
      {status === "loading" && t("unsubscribing")}
      {status === "done" && t("unsubscribed")}
      {status === "failed" && t("unsubscribeFailed")}
      {status === "idle" && t("unsubscribe")}
    </button>
  );
}
