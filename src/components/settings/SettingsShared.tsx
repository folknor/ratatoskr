import type React from "react";

export function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}): React.ReactNode {
  return (
    <div>
      <h3 className="text-xs font-semibold uppercase tracking-wider text-text-tertiary mb-3">
        {title}
      </h3>
      <div className="space-y-3">{children}</div>
    </div>
  );
}

export function SettingRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}): React.ReactNode {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-text-secondary">{label}</span>
      {children}
    </div>
  );
}

export function ToggleRow({
  label,
  description,
  checked,
  onToggle,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onToggle: () => void;
}): React.ReactNode {
  return (
    <div className="flex items-center justify-between">
      <div>
        <span className="text-sm text-text-secondary">{label}</span>
        {description != null && (
          <p className="text-xs text-text-tertiary mt-0.5">{description}</p>
        )}
      </div>
      <button
        type="button"
        onClick={onToggle}
        className={`w-10 h-5 rounded-full transition-colors relative shrink-0 ml-4 ${
          checked ? "bg-accent" : "bg-bg-tertiary"
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full transition-transform shadow ${
            checked ? "translate-x-5" : ""
          }`}
        />
      </button>
    </div>
  );
}

export function InfoRow({
  label,
  value,
}: {
  label: string;
  value: string;
}): React.ReactNode {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-text-secondary">{label}</span>
      <span className="text-sm text-text-primary font-mono">{value}</span>
    </div>
  );
}
