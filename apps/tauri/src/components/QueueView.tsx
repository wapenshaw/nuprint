import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { JobSummary } from "../bindings/JobSummary";
import { formatMm } from "../lib/estimate";
import { formatDateTime, pluralize } from "../lib/format";
import { isCancellable, isTerminal } from "../lib/labels";
import { newIdempotencyKey } from "../lib/uuid";
import { useApp } from "../state/AppStore";
import { useJobActions } from "../state/useJobActions";
import { CloseIcon } from "./icons";
import { JobDetails } from "./JobDetails";
import { ReceiptThumb } from "./Receipt";
import { Modal } from "./ui";

/**
 * The queue, slid in from the right over the workspace. Not modal: the toolbar and status bar
 * stay usable. Escape, the close button or a click on the dimmed workspace closes it.
 */
export function QueuePanel() {
  const { data, notify, refreshQueue, setView } = useApp();
  const { busy, run, cancel } = useJobActions();
  const [confirmClear, setConfirmClear] = useState(false);
  const panel = useRef<HTMLElement>(null);
  const close = () => setView("workspace");

  // Focus moves into the panel and back to where it was (the Queue button) when it closes.
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    panel.current?.focus();
    return () => {
      if (previous?.isConnected) previous.focus();
    };
  }, []);

  const hasHeldJobs = data.jobs.some((j) => j.state === "held");
  const hasHistory = data.jobs.some((j) => isTerminal(j.state));

  async function toggleHold() {
    await run("hold", async () => {
      const snap = data.queueHeld ? await api().resumeQueue() : await api().holdQueue();
      await refreshQueue().catch(() => undefined);
      notify(snap.held ? "Queue held. Waiting jobs will not start until you resume." : "Queue resumed.", "info");
    });
  }

  const waiting = data.jobs.filter((j) => isCancellable(j.state)).length;

  return (
    <>
      <div className="drawer-scrim" aria-hidden="true" onMouseDown={close} />
      <aside
        id="queue-panel"
        ref={panel}
        className="drawer"
        aria-labelledby="queue-heading"
        tabIndex={-1}
        onKeyDown={(e) => {
          if (e.key === "Escape" && !e.defaultPrevented) {
            e.preventDefault();
            close();
          }
        }}
      >
      <div className="drawer-head">
        <h2 id="queue-heading">Queue</h2>
        <span className="muted">{waiting > 0 ? `${waiting} not finished` : ""}</span>
        <button
          type="button"
          className="btn btn-small"
          onClick={() => void toggleHold()}
          disabled={busy === "hold" || data.queueFault !== null}
        >
          {data.queueHeld ? "Resume queue" : "Hold queue"}
        </button>
        <button type="button" className="btn btn-quiet icon-only btn-small" onClick={close} aria-label="Close queue">
          <CloseIcon />
        </button>
      </div>
      <div className="drawer-body">

      {data.queueFault ? (
        <p className="note note-error" role="alert">
          <strong>Queue stopped: </strong>
          {data.queueFault.message}
        </p>
      ) : null}
      {data.queueHeld && !data.queueFault ? (
        <p className="note note-warn" role="status">
          <strong>Queue held: </strong>A job that is already printing will finish or can be cancelled, but waiting jobs
          will not start until you resume.
        </p>
      ) : null}
      {hasHeldJobs && !data.queueFault ? (
        <p className="note note-warn" role="status">
          <strong>Jobs were held after restart: </strong>resume when ready. NuPrint never replays a half-sent job on
          its own, so check each job first.
        </p>
      ) : null}

      {data.jobs.length === 0 ? (
        <div className="empty-block">
          <p>
            <strong>No jobs yet.</strong> Press Print and the job appears here while it prints.
          </p>
        </div>
      ) : (
        <ul className="job-list" aria-label="Print jobs, newest first">
          {data.jobs.map((job) => (
            <JobRow key={job.id} job={job} onRun={run} onCancel={cancel} disabled={busy !== null} />
          ))}
        </ul>
      )}
      </div>
      <div className="drawer-foot">
        <span className="hint">Printed jobs keep their details and a small preview, not their print data.</span>
        <button type="button" className="btn btn-small" onClick={() => setConfirmClear(true)} disabled={!hasHistory}>
          Clear history
        </button>
      </div>

      {confirmClear ? (
        <Modal
          title="Clear history?"
          onClose={() => setConfirmClear(false)}
          footer={
            <>
              <button type="button" className="btn" onClick={() => setConfirmClear(false)}>
                Keep history
              </button>
              <button
                type="button"
                className="btn btn-danger"
                onClick={() => {
                  setConfirmClear(false);
                  void run("clear", async () => {
                    await api().clearHistory();
                    await refreshQueue();
                    notify("History cleared.", "success");
                  });
                }}
              >
                Clear history
              </button>
            </>
          }
        >
          <p>
            This removes finished, failed and cancelled jobs from the list, along with any saved content for printing
            them again. Jobs that are waiting or printing stay.
          </p>
        </Modal>
      ) : null}
      </aside>
    </>
  );
}

function JobRow({
  job,
  onRun,
  onCancel,
  disabled,
}: {
  job: JobSummary;
  onRun: (label: string, fn: () => Promise<unknown>) => Promise<void>;
  onCancel: (job: JobSummary) => Promise<void>;
  disabled: boolean;
}) {
  const { notify } = useApp();
  const terminal = isTerminal(job.state);

  return (
    <li className={`job job-${job.state}`}>
      <ReceiptThumb job={job} />
      <JobDetails
        job={job}
        meta={
          <>
            {job.printer_name} · {pluralize(job.pages, "page")} × {pluralize(job.copies, "copy", "copies")} · ≈{" "}
            {formatMm(job.paper_mm)} · <span className="nowrap">{formatDateTime(job.created_ms)}</span>
            {job.retry_of ? " · reprint" : ""}
          </>
        }
      />
      <div className="job-actions">
        {isCancellable(job.state) ? (
          <button
            type="button"
            className="btn btn-small"
            disabled={disabled}
            aria-label={`Cancel ${job.title}`}
            onClick={() => void onCancel(job)}
          >
            Cancel
          </button>
        ) : null}
        {terminal && job.content_retained ? (
          <button
            type="button"
            className="btn btn-small"
            disabled={disabled}
            aria-label={`Print ${job.title} again as a new job`}
            onClick={() =>
              void onRun("retry", async () => {
                await api().retryJob(job.id, newIdempotencyKey());
                notify(`Added "${job.title}" again as a new job.`, "success");
              })
            }
          >
            Print again
          </button>
        ) : null}
        {terminal ? (
          <button
            type="button"
            className="btn btn-quiet btn-small"
            disabled={disabled}
            aria-label={`Remove ${job.title} from the list`}
            onClick={() => void onRun("remove", () => api().removeJob(job.id))}
          >
            Remove
          </button>
        ) : null}
      </div>
    </li>
  );
}
