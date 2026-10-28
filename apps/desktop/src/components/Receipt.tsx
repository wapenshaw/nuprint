import { useState } from "react";
import { api } from "../api";
import type { JobSummary } from "../bindings/JobSummary";
import { formatMm } from "../lib/estimate";

/**
 * A job in the queue drawn as the receipt it makes: its first page (the saved page while the job
 * waits or can be retried, a kept preview once it has printed), otherwise blank paper as long as
 * it used.
 */
export function ReceiptThumb({ job }: { job: JobSummary }) {
  const [broken, setBroken] = useState(false);
  const src = broken ? "" : api().jobPreviewUrl(job.id);
  // Blank paper: about 1 px per mm, kept within the thumbnail.
  const blankHeight = Math.min(84, Math.max(26, Math.round(job.paper_mm)));

  return (
    <div
      className="receipt-thumb"
      aria-hidden="true"
      title={src ? undefined : `≈ ${formatMm(job.paper_mm)} of paper. No preview of it is kept.`}
    >
      <div className="receipt-paper">
        {src ? (
          <img src={src} alt="" draggable={false} onError={() => setBroken(true)} />
        ) : (
          <div className="receipt-blank" style={{ height: blankHeight }} />
        )}
      </div>
    </div>
  );
}
