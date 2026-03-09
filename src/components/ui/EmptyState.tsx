import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";

type EmptyStateProps = {
  title: string;
  subtitle?: string;
} & (
  | { icon: LucideIcon; illustration?: never }
  | { illustration: ComponentType<{ size?: number; className?: string }>; icon?: never }
);

export function EmptyState({ title, subtitle, ...rest }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-text-tertiary px-4">
      {"illustration" in rest && rest.illustration ? (
        <rest.illustration size={140} className="mb-4 opacity-80" />
      ) : "icon" in rest && rest.icon ? (
        (() => { const Icon = rest.icon; return <Icon size={48} strokeWidth={1} className="mb-3 opacity-40" />; })()
      ) : null}
      <p className="text-sm font-medium">{title}</p>
      {subtitle && <p className="text-xs mt-1 text-center">{subtitle}</p>}
    </div>
  );
}
