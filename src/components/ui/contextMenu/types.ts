export interface MenuComponentProps {
  position: { x: number; y: number };
  data: Record<string, unknown>;
  onClose: () => void;
}

export interface ThreadMenuProps extends MenuComponentProps {
  onSnooze: (target: { threadIds: string[]; accountId: string }) => void;
}
