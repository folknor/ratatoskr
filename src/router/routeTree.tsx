import type { AnyRoute } from "@tanstack/react-router";
import { createRootRoute, createRoute, redirect } from "@tanstack/react-router";
import React, {
  type ComponentType,
  type LazyExoticComponent,
  lazy,
  Suspense,
} from "react";
import App from "@/App";
import { MailLayout } from "@/components/layout/MailLayout";
import { ErrorBoundary } from "@/components/ui/ErrorBoundary";

// Lazy-load heavy pages — these include many sub-components and service imports
const SettingsPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/components/settings/SettingsPage").then((m) => ({
    default: m.SettingsPage,
  })),
);
const HelpPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/components/help/HelpPage").then((m) => ({ default: m.HelpPage })),
);
const CalendarPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/components/calendar/CalendarPage").then((m) => ({
    default: m.CalendarPage,
  })),
);
const TasksPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/components/tasks/TasksPage").then((m) => ({
    default: m.TasksPage,
  })),
);
const AttachmentLibrary: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/components/attachments/AttachmentLibrary").then((m) => ({
    default: m.AttachmentLibrary,
  })),
);

// ---------- Search param validation ----------
const VALID_CATEGORIES: readonly [
  "Primary",
  "Updates",
  "Promotions",
  "Social",
  "Newsletters",
] = ["Primary", "Updates", "Promotions", "Social", "Newsletters"] as const;

type MailSearch = {
  q?: string;
  category?: (typeof VALID_CATEGORIES)[number];
};

function validateMailSearch(search: Record<string, unknown>): MailSearch {
  const result: MailSearch = {};
  if (typeof search["q"] === "string" && search["q"]) {
    result.q = search["q"];
  }
  const cat: unknown = search["category"];
  if (
    typeof cat === "string" &&
    (VALID_CATEGORIES as readonly string[]).includes(cat)
  ) {
    result.category = cat as MailSearch["category"];
  }
  return result;
}

// ---------- Root (shell: TitleBar, Sidebar, overlays) ----------
export const rootRoute: ReturnType<typeof createRootRoute> = createRootRoute({
  component: App,
});

// ---------- / (index) → redirect to /mail/inbox ----------
const indexRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/mail/$label", params: { label: "inbox" } });
  },
});

// ---------- Mail routes: render MailLayout for all mail views ----------
function MailPage(): React.JSX.Element {
  return (
    <ErrorBoundary name="MailLayout">
      <MailLayout />
    </ErrorBoundary>
  );
}

function SettingsTabPage(): React.JSX.Element {
  return (
    <ErrorBoundary name="SettingsPage">
      <Suspense
        fallback={
          <div className="flex-1 flex items-center justify-center text-text-tertiary text-sm">
            Loading settings...
          </div>
        }
      >
        <SettingsPage />
      </Suspense>
    </ErrorBoundary>
  );
}

function CalendarPageWrapper(): React.JSX.Element {
  return (
    <ErrorBoundary name="CalendarPage">
      <Suspense
        fallback={
          <div className="flex-1 flex items-center justify-center text-text-tertiary text-sm">
            Loading calendar...
          </div>
        }
      >
        <CalendarPage />
      </Suspense>
    </ErrorBoundary>
  );
}

function HelpPageWrapper(): React.JSX.Element {
  return (
    <ErrorBoundary name="HelpPage">
      <Suspense
        fallback={
          <div className="flex-1 flex items-center justify-center text-text-tertiary text-sm">
            Loading help...
          </div>
        }
      >
        <HelpPage />
      </Suspense>
    </ErrorBoundary>
  );
}

// ---------- /mail/$label ----------
export const mailRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "mail/$label",
  validateSearch: validateMailSearch,
  component: MailPage,
});

// ---------- /mail/$label/thread/$threadId ----------
export const mailThreadRoute: AnyRoute = createRoute({
  getParentRoute: () => mailRoute,
  path: "thread/$threadId",
});

// ---------- /label/$labelId ----------
export const labelRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "label/$labelId",
  validateSearch: validateMailSearch,
  component: MailPage,
});

// ---------- /label/$labelId/thread/$threadId ----------
export const labelThreadRoute: AnyRoute = createRoute({
  getParentRoute: () => labelRoute,
  path: "thread/$threadId",
});

// ---------- /smart-folder/$folderId ----------
export const smartFolderRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "smart-folder/$folderId",
  validateSearch: validateMailSearch,
  component: MailPage,
});

// ---------- /smart-folder/$folderId/thread/$threadId ----------
export const smartFolderThreadRoute: AnyRoute = createRoute({
  getParentRoute: () => smartFolderRoute,
  path: "thread/$threadId",
});

// ---------- /settings (redirect to /settings/general) ----------
const settingsIndexRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "settings",
  beforeLoad: () => {
    throw redirect({ to: "/settings/$tab", params: { tab: "general" } });
  },
});

// ---------- /settings/$tab ----------
export const settingsTabRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "settings/$tab",
  component: SettingsTabPage,
});

// ---------- /attachments ----------
function AttachmentLibraryWrapper(): React.JSX.Element {
  return (
    <ErrorBoundary name="AttachmentLibrary">
      <Suspense
        fallback={
          <div className="flex-1 flex items-center justify-center text-text-tertiary text-sm">
            Loading attachments...
          </div>
        }
      >
        <AttachmentLibrary />
      </Suspense>
    </ErrorBoundary>
  );
}

export const attachmentsRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "attachments",
  component: AttachmentLibraryWrapper,
});

// ---------- /tasks ----------
function TasksPageWrapper(): React.JSX.Element {
  return (
    <ErrorBoundary name="TasksPage">
      <Suspense
        fallback={
          <div className="flex-1 flex items-center justify-center text-text-tertiary text-sm">
            Loading tasks...
          </div>
        }
      >
        <TasksPage />
      </Suspense>
    </ErrorBoundary>
  );
}

export const tasksRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "tasks",
  component: TasksPageWrapper,
});

// ---------- /calendar ----------
export const calendarRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "calendar",
  component: CalendarPageWrapper,
});

// ---------- /help (redirect to /help/getting-started) ----------
const helpIndexRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "help",
  beforeLoad: () => {
    throw redirect({
      to: "/help/$topic",
      params: { topic: "getting-started" },
    });
  },
});

// ---------- /help/$topic ----------
export const helpTopicRoute: AnyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "help/$topic",
  component: HelpPageWrapper,
});

// ---------- Route tree ----------
export const routeTree: AnyRoute = rootRoute.addChildren([
  indexRoute,
  mailRoute.addChildren([mailThreadRoute]),
  labelRoute.addChildren([labelThreadRoute]),
  smartFolderRoute.addChildren([smartFolderThreadRoute]),
  settingsIndexRoute,
  settingsTabRoute,
  attachmentsRoute,
  tasksRoute,
  calendarRoute,
  helpIndexRoute,
  helpTopicRoute,
]);
