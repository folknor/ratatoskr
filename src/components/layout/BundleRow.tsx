import { ChevronRight, Package } from "lucide-react";
import type React from "react";
import type { DbBundleRule } from "@/services/db/bundleRules";
import type { Thread } from "@/stores/threadStore";
import { ThreadCard } from "../email/ThreadCard";

export function BundleRow({
  rule,
  summary,
  isExpanded,
  onToggle,
  bundledThreads,
  selectedThreadId,
  onThreadClick,
  onContextMenu,
  followUpThreadIds,
}: {
  rule: DbBundleRule;
  summary: {
    count: number;
    latestSubject: string | null;
    latestSender: string | null;
  };
  isExpanded: boolean;
  onToggle: () => void;
  bundledThreads: Thread[];
  selectedThreadId: string | null;
  onThreadClick: (thread: Thread) => void;
  onContextMenu: (e: React.MouseEvent, threadId: string) => void;
  followUpThreadIds: Set<string>;
}): React.ReactNode {
  if (summary.count === 0) return null;

  return (
    <div>
      <button
        type="button"
        onClick={onToggle}
        className="w-full text-left px-4 py-3 border-b border-border-secondary hover:bg-bg-hover transition-colors flex items-center gap-3"
      >
        <div className="w-9 h-9 rounded-full bg-accent/15 flex items-center justify-center shrink-0">
          <Package size={16} className="text-accent" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold text-text-primary">
              {rule.category}
            </span>
            <span className="text-xs bg-accent/15 text-accent px-1.5 rounded-full">
              {summary.count}
            </span>
          </div>
          <span className="text-xs text-text-tertiary truncate block mt-0.5">
            {summary.latestSender != null && `${summary.latestSender}: `}
            {summary.latestSubject ?? ""}
          </span>
        </div>
        <ChevronRight
          size={14}
          className={`text-text-tertiary transition-transform shrink-0 ${isExpanded ? "rotate-90" : ""}`}
        />
      </button>
      {isExpanded &&
        bundledThreads.map((thread) => (
          <div key={thread.id} className="pl-4">
            <ThreadCard
              thread={thread}
              isSelected={thread.id === selectedThreadId}
              onClick={onThreadClick}
              onContextMenu={onContextMenu}
              category={rule.category}
              hasFollowUp={followUpThreadIds.has(thread.id)}
            />
          </div>
        ))}
    </div>
  );
}
