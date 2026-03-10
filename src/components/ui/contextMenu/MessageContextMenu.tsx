import { Code, Copy, Forward, Reply, ReplyAll } from "lucide-react";
import type React from "react";
import { useComposerStore } from "@/stores/composerStore";
import { buildForwardQuote, buildQuote } from "@/utils/emailQuoteBuilders";
import { ContextMenu, type ContextMenuItem } from "../ContextMenu";
import type { MenuComponentProps } from "./types";

export function MessageContextMenu({
  position,
  data,
  onClose,
}: MenuComponentProps): React.ReactNode {
  const openComposer = useComposerStore((s) => s.openComposer);

  const messageId = data["messageId"] as string;
  const threadId = data["threadId"] as string;
  const accountId = data["accountId"] as string | null;
  const fromAddress = data["fromAddress"] as string | null;
  const fromName = data["fromName"] as string | null;
  const replyTo = data["replyTo"] as string | null;
  const toAddresses = data["toAddresses"] as string | null;
  const ccAddresses = data["ccAddresses"] as string | null;
  const subject = data["subject"] as string | null;
  const date = data["date"] as string | number;
  const bodyHtml = data["bodyHtml"] as string | null;
  const bodyText = data["bodyText"] as string | null;

  const msg = {
    from_name: fromName,
    from_address: fromAddress,
    date,
    body_html: bodyHtml,
    body_text: bodyText,
    subject,
    to_addresses: toAddresses,
  };

  const handleReply = (): void => {
    const replyAddr = replyTo ?? fromAddress;
    openComposer({
      mode: "reply",
      to: replyAddr ? [replyAddr] : [],
      subject: `Re: ${subject ?? ""}`,
      bodyHtml: buildQuote(msg),
      threadId,
      inReplyToMessageId: messageId,
    });
  };

  const handleReplyAll = (): void => {
    const replyAddr = replyTo ?? fromAddress;
    const allRecipients = new Set<string>();
    if (replyAddr) allRecipients.add(replyAddr);
    if (toAddresses) {
      for (const a of toAddresses.split(",")) {
        allRecipients.add(a.trim());
      }
    }
    const ccList: string[] = [];
    if (ccAddresses) {
      for (const a of ccAddresses.split(",")) {
        ccList.push(a.trim());
      }
    }
    openComposer({
      mode: "replyAll",
      to: Array.from(allRecipients),
      cc: ccList,
      subject: `Re: ${subject ?? ""}`,
      bodyHtml: buildQuote(msg),
      threadId,
      inReplyToMessageId: messageId,
    });
  };

  const handleForward = (): void => {
    openComposer({
      mode: "forward",
      to: [],
      subject: `Fwd: ${subject ?? ""}`,
      bodyHtml: buildForwardQuote(msg),
      threadId,
      inReplyToMessageId: messageId,
    });
  };

  const handleCopy = async (): Promise<void> => {
    const text = bodyText ?? "";
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Fallback: no-op in non-secure contexts
    }
  };

  const items: ContextMenuItem[] = [
    {
      id: "reply",
      label: "Reply",
      icon: Reply,
      shortcut: "r",
      action: handleReply,
    },
    {
      id: "reply-all",
      label: "Reply All",
      icon: ReplyAll,
      shortcut: "a",
      action: handleReplyAll,
    },
    {
      id: "forward",
      label: "Forward",
      icon: Forward,
      shortcut: "f",
      action: handleForward,
    },
    { id: "sep-1", label: "", separator: true },
    {
      id: "copy-text",
      label: "Copy Message Text",
      icon: Copy,
      action: handleCopy,
    },
    ...(accountId
      ? [
          { id: "sep-2", label: "", separator: true },
          {
            id: "view-source",
            label: "View Source",
            icon: Code,
            action: () => {
              window.dispatchEvent(
                new CustomEvent("ratatoskr-view-raw-message", {
                  detail: { messageId, accountId },
                }),
              );
            },
          },
        ]
      : []),
  ];

  return <ContextMenu items={items} position={position} onClose={onClose} />;
}
