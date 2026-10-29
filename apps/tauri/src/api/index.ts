import { createMockBackend, SAMPLE_UPDATE } from "./mock";
import { createTauriBackend } from "./tauri";
import type { Backend } from "./types";

export type { Backend, Unsubscribe } from "./types";

export const isTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let backend: Backend | null = null;

/**
 * The active backend. Inside Tauri it is the real host; otherwise (plain browser via
 * `npm run dev`, or vitest) it is the in-memory mock. Add `?mock=empty` to the URL to
 * start the mock with no saved printers, or `?mock=update` for a check to find a newer release.
 */
export function api(): Backend {
  if (!backend) {
    if (isTauri()) {
      backend = createTauriBackend();
    } else {
      const flag = typeof location !== "undefined" ? new URLSearchParams(location.search).get("mock") : null;
      backend = createMockBackend({ seedPrinters: flag !== "empty", update: flag === "update" ? SAMPLE_UPDATE : null });
    }
  }
  return backend;
}

/** Replace the backend (tests). */
export function setBackend(next: Backend | null): void {
  backend = next;
}
