import React from "react";
import ReactDOM from "react-dom/client";
import "@vscode/codicons/dist/codicon.css";
import "allotment/dist/style.css";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import App from "./App";
import { api } from "./api";
import { ErrorBoundary } from "./components/ErrorBoundary";

function formatUnknownReason(value: unknown): { message: string; stack?: string } {
  if (value instanceof Error) {
    return {
      message: value.message || value.name || "Unknown Error",
      stack: value.stack,
    };
  }
  if (typeof value === "string") {
    return { message: value };
  }
  try {
    return { message: JSON.stringify(value) };
  } catch {
    return { message: String(value) };
  }
}

function installGlobalCrashHooks(): void {
  if (typeof window === "undefined") return;

  window.addEventListener("error", (event) => {
    const err = event.error instanceof Error ? event.error : null;
    const loc = event.filename
      ? ` (${event.filename}:${event.lineno}:${event.colno})`
      : "";
    const msg = `${event.message || err?.message || "Uncaught window error"}${loc}`;
    api.frontendTraceLog("error", "window.error", msg, err?.stack ?? null);
  });

  window.addEventListener("unhandledrejection", (event) => {
    const info = formatUnknownReason(event.reason);
    api.frontendTraceLog(
      "error",
      "window.unhandledrejection",
      info.message,
      info.stack ?? null,
    );
  });

  const originalConsoleError = console.error.bind(console);
  let inConsoleErrorHook = false;
  console.error = (...args: unknown[]) => {
    originalConsoleError(...args);
    if (inConsoleErrorHook) return;
    inConsoleErrorHook = true;
    try {
      const parts: string[] = [];
      let stack: string | undefined;
      for (const arg of args) {
        const info = formatUnknownReason(arg);
        parts.push(info.message);
        if (!stack && info.stack) {
          stack = info.stack;
        }
      }
      api.frontendTraceLog(
        "error",
        "console.error",
        parts.join(" ") || "console.error invoked",
        stack ?? null,
      );
    } finally {
      inConsoleErrorHook = false;
    }
  };
}

installGlobalCrashHooks();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary scope="root-shell" title="PackStudio UI encountered an unexpected error">
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
