import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Thread } from "@/stores/threadStore";
import { ThreadCard } from "./ThreadCard";

vi.mock("@dnd-kit/core", () => ({
  useDraggable: () => ({
    attributes: {},
    listeners: {},
    setNodeRef: vi.fn(),
    isDragging: false,
  }),
}));

vi.mock("@/stores/threadStore", () => ({
  useThreadStore: Object.assign(
    (selector: (s: Record<string, unknown>) => unknown) =>
      selector({
        selectedThreadIds: new Set(),
        toggleThreadSelection: vi.fn(),
        selectThreadRange: vi.fn(),
      }),
    { getState: () => ({ selectedThreadIds: new Set() }) },
  ),
}));

vi.mock("@/stores/uiPreferencesStore", () => ({
  useUIPreferencesStore: (selector: (s: Record<string, unknown>) => unknown) =>
    selector({ emailDensity: "default" }),
}));

vi.mock("@/hooks/useRouteNavigation", () => ({
  useActiveLabel: () => "inbox",
}));

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "t1",
    accountId: "a1",
    subject: "Test subject",
    snippet: "Test snippet",
    lastMessageAt: Date.now(),
    messageCount: 1,
    isRead: false,
    isStarred: false,
    isPinned: false,
    isMuted: false,
    hasAttachments: false,
    labelIds: ["INBOX"],
    fromName: "Alice",
    fromAddress: "alice@example.com",
    ...overrides,
  };
}

describe("ThreadCard", () => {
  const onClick = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders sender name and subject", () => {
    render(
      <ThreadCard thread={makeThread()} isSelected={false} onClick={onClick} />,
    );
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("Test subject")).toBeInTheDocument();
  });

  it("applies red background for spam threads", () => {
    const { container } = render(
      <ThreadCard
        thread={makeThread({ labelIds: ["SPAM"] })}
        isSelected={false}
        onClick={onClick}
      />,
    );
    const button = container.querySelector("button") as Element;
    expect(button.className).toContain("bg-red-500/8");
  });

  it("does not apply red background for non-spam threads", () => {
    const { container } = render(
      <ThreadCard
        thread={makeThread({ labelIds: ["INBOX"] })}
        isSelected={false}
        onClick={onClick}
      />,
    );
    const button = container.querySelector("button") as Element;
    expect(button.className).not.toContain("bg-red-500");
  });

  it("applies red background for spam even when thread has other labels", () => {
    const { container } = render(
      <ThreadCard
        thread={makeThread({ labelIds: ["INBOX", "SPAM", "IMPORTANT"] })}
        isSelected={false}
        onClick={onClick}
      />,
    );
    const button = container.querySelector("button") as Element;
    expect(button.className).toContain("bg-red-500/8");
  });
});
