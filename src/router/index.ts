import type { RouterHistory } from "@tanstack/react-router";
import { createHashHistory, createRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree";

const hashHistory: RouterHistory = createHashHistory();

export const router: ReturnType<typeof createRouter> = createRouter({
  routeTree,
  history: hashHistory,
  defaultPreload: false,
});

// Type-safe router module augmentation
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
