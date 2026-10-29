import type { ReactNode } from "react";
import type { JobSummary } from "../bindings/JobSummary";
import { formatBytes } from "../lib/format";
import { JOB_STATE_LABEL, JOB_STATE_TONE, isActive } from "../lib/labels";
import { useApp } from "../state/AppStore";
import { Badge } from "./ui";

/** Title, state, transfer progress and problems of one job; `meta` is the summary line. */
export function JobDetails({ job, meta }: { job: JobSummary; meta: ReactNode }) {
  const label = JOB_STATE_LABEL[job.state];
  return (
    <div className="job-main">
      <div className="job-title-row">
        <span className="job-title">{job.title}</span>
        <Badge tone={JOB_STATE_TONE[job.state]}>{label}</Badge>
      </div>
      <p className="job-meta">{meta}</p>
      {job.state_text && job.state_text !== label ? <p className="job-text">{job.state_text}</p> : null}
      <JobProgress job={job} />
      {job.error ? (
        <p className="note note-error" role="alert">
          <strong>Error: </strong>
          {job.error.message}
        </p>
      ) : null}
      {job.partial_output_possible ? (
        <p className="note note-warn">
          <strong>Some paper may have printed.</strong> Check the paper before printing again.
        </p>
      ) : null}
    </div>
  );
}

/** Byte-transfer bar for an active job; renders nothing when there is no transfer to show. */
export function JobProgress({ job, compact = false }: { job: JobSummary; compact?: boolean }) {
  const { data } = useApp();
  const live = data.progress[job.id];
  const sent = Math.max(job.bytes_sent, live?.sent ?? 0);
  const total = live?.total ?? job.bytes_total;
  const show = job.state === "sending" || (isActive(job.state) && sent > 0 && total > 0);
  if (!show) return null;

  const detail = `Data transfer: ${formatBytes(sent)} of ${formatBytes(total)}. This is not page progress.`;
  if (compact) {
    return (
      <progress
        className="progress-compact"
        value={sent}
        max={Math.max(total, 1)}
        title={detail}
        aria-label={`Data sent to printer for ${job.title}`}
      />
    );
  }

  return (
    <div className="progress-wrap">
      <progress value={sent} max={Math.max(total, 1)} aria-label={`Data sent to printer for ${job.title}`} />
      <span className="hint">{detail}</span>
    </div>
  );
}
