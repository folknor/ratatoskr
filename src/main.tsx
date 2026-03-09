import { RouterProvider } from "@tanstack/react-router";
import type React from "react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import ComposerWindow from "./ComposerWindow";
import { router } from "./router";
import ThreadWindow from "./ThreadWindow";
import "./i18n";
import "./styles/globals.css";

const params: URLSearchParams = new URLSearchParams(window.location.search);
const isThreadWindow: boolean = params.has("thread") && params.has("account");
const isComposerWindow: boolean = params.has("compose");

function Root(): React.ReactNode {
  if (isThreadWindow) return <ThreadWindow />;
  if (isComposerWindow) return <ComposerWindow />;
  return <RouterProvider router={router} />;
}

const rootEl: HTMLElement | null = document.getElementById("root");
if (!rootEl) throw new Error("Root element not found");

createRoot(rootEl).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
