import { useCallback, useState } from "react";
import { api } from "../api";
import type { JobSummary } from "../bindings/JobSummary";
import { useApp } from "./AppStore";

/**
 * Queue actions with one busy flag: run an action, refresh the queue, report failures.
 * `busy` is the label of the running action, or null.
 */
export function useJobActions() {
  const { notify, reportError, refreshQueue } = useApp();
  const [busy, setBusy] = useState<string | null>(null);

  const run = useCallback(
    async (label: string, fn: () => Promise<unknown>) => {
      setBusy(label);
      try {
        await fn();
        await refreshQueue().catch(() => undefined);
      } catch (e) {
        reportError(e);
      } finally {
        setBusy(null);
      }
    },
    [refreshQueue, reportError],
  );

  const cancel = useCallback(
    /** `keepShown` keeps the job on the status bar, which is where it is being watched. */
    (job: JobSummary, keepShown = false) =>
      run("cancel", async () => {
        await api().cancelJob(job.id);
        notify(`Cancelling "${job.title}"…`, "info", undefined, keepShown ? job.id : undefined);
      }),
    [notify, run],
  );

  return { busy, run, cancel };
}
