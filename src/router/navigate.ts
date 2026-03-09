import { router } from "./index";

/** Known system labels that map to /mail/$label */
const SYSTEM_LABELS: Set<string> = new Set([
  "inbox",
  "starred",
  "snoozed",
  "sent",
  "drafts",
  "trash",
  "spam",
  "all",
]);

/**
 * Navigate to a label/view. Handles routing for system labels, custom labels,
 * smart folders, and special views (settings, calendar).
 */
export function navigateToLabel(
  label: string,
  opts?: { category?: string; threadId?: string },
): void {
  if (label === "settings") {
    router.navigate({ to: "/settings/$tab", params: { tab: "general" } });
    return;
  }

  if (label === "tasks") {
    router.navigate({ to: "/tasks" });
    return;
  }

  if (label === "attachments") {
    router.navigate({ to: "/attachments" });
    return;
  }

  if (label === "calendar") {
    router.navigate({ to: "/calendar" });
    return;
  }

  if (label === "help") {
    router.navigate({
      to: "/help/$topic",
      params: { topic: "getting-started" },
    });
    return;
  }

  if (label.startsWith("smart-folder:")) {
    const folderId: string = label.replace("smart-folder:", "");
    if (opts?.threadId) {
      router.navigate({
        to: "/smart-folder/$folderId/thread/$threadId",
        params: { folderId, threadId: opts.threadId },
      });
    } else {
      router.navigate({
        to: "/smart-folder/$folderId",
        params: { folderId },
      });
    }
    return;
  }

  if (SYSTEM_LABELS.has(label)) {
    const search: Record<string, string> = {};
    if (opts?.category) search["category"] = opts.category;
    if (opts?.threadId) {
      router.navigate({
        to: "/mail/$label/thread/$threadId",
        params: { label, threadId: opts.threadId },
        search,
      });
    } else {
      router.navigate({
        to: "/mail/$label",
        params: { label },
        search,
      });
    }
    return;
  }

  // Custom user label
  if (opts?.threadId) {
    router.navigate({
      to: "/label/$labelId/thread/$threadId",
      params: { labelId: label, threadId: opts.threadId },
    });
  } else {
    router.navigate({
      to: "/label/$labelId",
      params: { labelId: label },
    });
  }
}

/**
 * Navigate to a thread within the current mail context.
 * Appends /thread/$threadId to the current route.
 */
export function navigateToThread(threadId: string): void {
  const { location } = router.state;
  const pathname: string = location.pathname;

  // Already on a mail/$label route
  const mailMatch: RegExpMatchArray | null = pathname.match(/^\/mail\/([^/]+)/);
  if (mailMatch) {
    const label: string | undefined = mailMatch[1];
    if (label) {
      router.navigate({
        to: "/mail/$label/thread/$threadId",
        params: { label, threadId },
        search: location.search as Record<string, string>,
      });
      return;
    }
  }

  // On a custom label route
  const labelMatch: RegExpMatchArray | null =
    pathname.match(/^\/label\/([^/]+)/);
  if (labelMatch) {
    const labelId: string | undefined = labelMatch[1];
    if (labelId) {
      router.navigate({
        to: "/label/$labelId/thread/$threadId",
        params: { labelId, threadId },
        search: location.search as Record<string, string>,
      });
      return;
    }
  }

  // On a smart folder route
  const sfMatch: RegExpMatchArray | null = pathname.match(
    /^\/smart-folder\/([^/]+)/,
  );
  if (sfMatch) {
    const folderId: string | undefined = sfMatch[1];
    if (folderId) {
      router.navigate({
        to: "/smart-folder/$folderId/thread/$threadId",
        params: { folderId, threadId },
        search: location.search as Record<string, string>,
      });
      return;
    }
  }

  // Fallback: navigate to inbox with thread
  router.navigate({
    to: "/mail/$label/thread/$threadId",
    params: { label: "inbox", threadId },
  });
}

/**
 * Navigate to settings with an optional tab.
 */
export function navigateToSettings(tab: string = "general"): void {
  router.navigate({ to: "/settings/$tab", params: { tab } });
}

/**
 * Navigate to help with an optional topic.
 */
export function navigateToHelp(topic: string = "getting-started"): void {
  router.navigate({ to: "/help/$topic", params: { topic } });
}

/**
 * Navigate back (deselect thread → go to parent list route).
 */
export function navigateBack(): void {
  const { location } = router.state;
  const pathname: string = location.pathname;

  // If on a thread sub-route, go to parent
  const mailThreadMatch: RegExpMatchArray | null = pathname.match(
    /^\/mail\/([^/]+)\/thread\//,
  );
  if (mailThreadMatch) {
    const label: string | undefined = mailThreadMatch[1];
    if (label) {
      router.navigate({
        to: "/mail/$label",
        params: { label },
        search: location.search as Record<string, string>,
      });
      return;
    }
  }

  const labelThreadMatch: RegExpMatchArray | null = pathname.match(
    /^\/label\/([^/]+)\/thread\//,
  );
  if (labelThreadMatch) {
    const labelId: string | undefined = labelThreadMatch[1];
    if (labelId) {
      router.navigate({
        to: "/label/$labelId",
        params: { labelId },
        search: location.search as Record<string, string>,
      });
      return;
    }
  }

  const sfThreadMatch: RegExpMatchArray | null = pathname.match(
    /^\/smart-folder\/([^/]+)\/thread\//,
  );
  if (sfThreadMatch) {
    const folderId: string | undefined = sfThreadMatch[1];
    if (folderId) {
      router.navigate({
        to: "/smart-folder/$folderId",
        params: { folderId },
        search: location.search as Record<string, string>,
      });
      return;
    }
  }

  // Not on a thread route — navigate to inbox
  router.navigate({ to: "/mail/$label", params: { label: "inbox" } });
}

/**
 * Get the active label from the current router state (non-React helper).
 */
export function getActiveLabel(): string {
  const matches: Array<{ routeId: string; params: Record<string, unknown> }> =
    router.state.matches;
  for (const match of matches) {
    if (
      match.routeId === "/mail/$label" ||
      match.routeId === "/mail/$label/thread/$threadId"
    ) {
      return (match.params as { label: string }).label;
    }
    if (
      match.routeId === "/label/$labelId" ||
      match.routeId === "/label/$labelId/thread/$threadId"
    ) {
      return (match.params as { labelId: string }).labelId;
    }
    if (
      match.routeId === "/smart-folder/$folderId" ||
      match.routeId === "/smart-folder/$folderId/thread/$threadId"
    ) {
      return `smart-folder:${(match.params as { folderId: string }).folderId}`;
    }
    if (match.routeId === "/settings/$tab" || match.routeId === "/settings") {
      return "settings";
    }
    if (match.routeId === "/attachments") {
      return "attachments";
    }
    if (match.routeId === "/tasks") {
      return "tasks";
    }
    if (match.routeId === "/calendar") {
      return "calendar";
    }
    if (match.routeId === "/help/$topic" || match.routeId === "/help") {
      return "help";
    }
  }
  return "inbox";
}

/**
 * Get the selected thread ID from the current router state (non-React helper).
 */
export function getSelectedThreadId(): string | null {
  const matches: Array<{ params: Record<string, string> }> =
    router.state.matches;
  for (const match of matches) {
    const params: Record<string, string> = match.params;
    if (params["threadId"]) {
      return params["threadId"];
    }
  }
  return null;
}
