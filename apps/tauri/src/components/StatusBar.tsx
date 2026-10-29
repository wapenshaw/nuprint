import type { JobSummary } from "../bindings/JobSummary";
import { JOB_STATE_LABEL, JOB_STATE_TONE, isCancellable, isOpenJob } from "../lib/labels";
import { useApp } from "../state/AppStore";
import { useJobActions } from "../state/useJobActions";
import { JobProgress } from "./JobDetails";
import { Badge } from "./ui";

const PREFIX = { info: "Info", success: "Done", error: "Error" } as const;

/**
 * One persistent status line. The live region itself stays mounted so screen readers
 * announce each new message; text carries the meaning, colour only reinforces it.
 * A message that names a job also shows that job's state and transfer progress, so the
 * latest print stays visible here until the next message; the queue keeps the history.
 * The bar is one fixed-height row, so nothing above it moves while a job runs.
 */
export function StatusBar() {
  const { data, dismissStatus, setView } = useApp();
  const { busy, cancel } = useJobActions();
  const s = data.status;
  const job = s?.jobId ? data.jobs.find((j) => j.id === s.jobId) : undefined;
  const tone = job && JOB_STATE_TONE[job.state] === "bad" ? "error" : (s?.tone ?? "info");
  // One line, so the bar never changes height; the Queue has the full details.
  const jobLine = job ? statusJobLine(job, data.queueHeld) : "";
  return (
    <footer className="status-bar">
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className={`status-text status-${tone}`}
        title={s ? `${PREFIX[tone]}: ${s.text}` : undefined}
      >
        {s ? (
          <>
            <strong>{PREFIX[tone]}: </strong>
            {s.text}
          </>
        ) : (
          <span className="muted">Ready. Keep the printer's phone app closed while NuPrint prints.</span>
        )}
      </div>
      {job ? (
        <div className="status-job" aria-label="Latest print job" title={jobLine}>
          <Badge tone={JOB_STATE_TONE[job.state]}>{JOB_STATE_LABEL[job.state]}</Badge>
          <span className="status-job-text">{jobLine}</span>
          <JobProgress job={job} compact />
          {job.error ? (
            <span className="sr-only" role="alert">
              Error: {job.error.message}
            </span>
          ) : null}
        </div>
      ) : null}
      {job && isCancellable(job.state) ? (
        <button
          type="button"
          className="btn btn-small"
          disabled={busy !== null}
          aria-label={`Cancel ${job.title}`}
          onClick={() => void cancel(job, true)}
        >
          Cancel
        </button>
      ) : null}
      {s?.action ? (
        <button
          type="button"
          className="btn btn-small"
          onClick={() => {
            setView(s.action!.target);
            dismissStatus();
          }}
        >
          {s.action.label}
        </button>
      ) : null}
      {s ? (
        <button type="button" className="btn btn-quiet btn-small" onClick={dismissStatus} aria-label="Dismiss message">
          Dismiss
        </button>
      ) : null}
    </footer>
  );
}

function statusJobLine(job: JobSummary, queueHeld: boolean): string {
  const parts = [job.printer_name];
  if (job.error) parts.push(job.error.message);
  else if (job.state_text && job.state_text !== JOB_STATE_LABEL[job.state]) parts.push(job.state_text);
  if (queueHeld && isOpenJob(job.state)) parts.push("Queue held: resume it in Queue");
  if (job.partial_output_possible) parts.push("Some paper may have printed; check it before printing again");
  return parts.join(" · ");
}
