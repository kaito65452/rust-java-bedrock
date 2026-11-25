import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";

interface TauriEventInternals {
  unregisterListener?: (event: string, eventId: number) => void;
}

/**
 * Safely invokes a Tauri `UnlistenFn` without throwing or creating an unhandled rejection
 * when WebKitGTK / Tauri internals have already torn down `unregisterListener` during unmount.
 */
export function safeUnlisten(unlisten: UnlistenFn | null | undefined): void {
  if (typeof unlisten !== "function") return;
  if (typeof window === "undefined") return;
  const internals = (
    window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__?: TauriEventInternals }
  ).__TAURI_EVENT_PLUGIN_INTERNALS__;
  if (typeof internals?.unregisterListener !== "function") {
    return;
  }
  try {
    const maybePromise = unlisten() as unknown;
    if (
      maybePromise &&
      typeof (maybePromise as Promise<unknown>).catch === "function"
    ) {
      void (maybePromise as Promise<unknown>).catch(() => {});
    }
  } catch {
    // Ignore already-unregistered or torn-down listener errors on unmount.
  }
}

/**
 * Subscribes to a Tauri event with cancellation tracking and `safeUnlisten` cleanup,
 * preventing React unmount / StrictMode double-mount `unregisterListener` unhandled rejections.
 */
export function safeListen<T>(
  eventName: string,
  handler: EventCallback<T>,
): () => void {
  let cancelled = false;
  let unlistenFn: UnlistenFn | null = null;

  void listen<T>(eventName, (ev) => {
    if (!cancelled) {
      handler(ev);
    }
  })
    .then((fn) => {
      if (cancelled) {
        safeUnlisten(fn);
      } else {
        unlistenFn = fn;
      }
    })
    .catch(() => {
      // Ignore transient listener registration failures (e.g. non-Tauri preview).
    });

  return () => {
    cancelled = true;
    const fn = unlistenFn;
    unlistenFn = null;
    safeUnlisten(fn);
  };
}
