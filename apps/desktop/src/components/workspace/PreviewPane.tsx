import { Fragment, useEffect, useRef, useState, type CSSProperties } from "react";
import { api } from "../../api";
import { formatMm } from "../../lib/estimate";
import { useApp } from "../../state/AppStore";
import { useWorkspace } from "../../state/WorkspaceStore";
import { emptyHint } from "../../state/workspace";
import { shortcut } from "../../lib/format";
import { SaveIcon } from "../icons";
import { ErrorNote, Segmented } from "../ui";

type Zoom = "fit" | "actual";

const ZOOMS: { id: Zoom; label: string; title?: string }[] = [
  { id: "actual", label: "Actual size", title: "One printer dot per screen pixel" },
  { id: "fit", label: "Fit width" },
];

/** Blank paper drawn either side of the printed image, in dots. Visual only. */
const PAPER_EDGE_DOTS = 16;

export function PreviewPane() {
  const { previewProfile, selectedPrinter } = useApp();
  const { ws, preview, savePng, settings, printing } = useWorkspace();
  const [zoom, setZoom] = useState<Zoom>("actual");
  // Each Print press feeds the paper out of the slot once (skipped with reduced motion).
  const [feeds, setFeeds] = useState(0);
  const wasPrinting = useRef(false);
  useEffect(() => {
    if (printing && !wasPrinting.current) setFeeds((n) => n + 1);
    wasPrinting.current = printing;
  }, [printing]);
  const { result, rendering, error, idle, current } = preview;
  const copies = settings?.copies ?? 1;

  let body;
  if (!previewProfile) {
    body = (
      <p className="empty">
        {selectedPrinter
          ? "This printer's profile is not available in this version of NuPrint, so a preview cannot be drawn."
          : "Loading printer profiles…"}
      </p>
    );
  } else if (idle) {
    body = <p className="empty">{emptyHint(ws.mode)}</p>;
  } else if (error && !rendering) {
    body = <ErrorNote>{error.message}</ErrorNote>;
  } else if (result) {
    // The paper as it comes out: every copy back to back, then the extra feed once as blank
    // paper, then the torn edge. The ruler measures it in millimetres from the first row.
    const dpm = previewProfile.dots_per_mm;
    const feedDots = Math.round((settings?.feedMm ?? 0) * dpm);
    const runDots = copies * result.height + feedDots;
    const tens = Array.from({ length: Math.floor(runDots / dpm / 10) }, (_, i) => (i + 1) * 10);
    const dots = {
      "--img-dots": result.width,
      "--edge-dots": PAPER_EDGE_DOTS,
      "--feed-dots": feedDots,
      "--run-dots": runDots,
      "--dpm": dpm,
    } as CSSProperties;
    const src = api().previewUrl(result.preview_path);
    body = (
      <div className="feed-out" style={dots}>
        <div className="paper-slot" aria-hidden="true" />
        <div className="paper-run">
          <div className="ruler" aria-hidden="true">
            {tens.map((mm) => (
              <span key={mm} style={{ "--at": mm } as CSSProperties}>
                {mm}
              </span>
            ))}
          </div>
          <div key={feeds} className={`paper-sheet${feeds > 0 ? " is-feeding" : ""}`}>
            <div className="paper" data-testid="paper">
              {Array.from({ length: copies }, (_, i) => (
                <Fragment key={i}>
                  {i > 0 ? <div className="copy-cut" aria-hidden="true" /> : null}
                  <img
                    className="preview-img"
                    src={src}
                    width={result.width}
                    height={result.height}
                    alt={i === 0 ? `Print preview, ${result.width} by ${result.height} dots` : ""}
                    draggable={false}
                  />
                </Fragment>
              ))}
            </div>
          </div>
        </div>
      </div>
    );
  } else {
    body = <p className="empty">Rendering…</p>;
  }

  return (
    <section className={`preview-desk${rendering && result ? " is-stale" : ""}`} aria-labelledby="preview-heading">
      <div className="desk-head">
        <h2 id="preview-heading">Paper preview</h2>
        <div className="desk-tools">
          <Segmented label="Preview size" hideLabel value={zoom} options={ZOOMS} onChange={setZoom} />
          <button
            type="button"
            className="btn"
            onClick={() => void savePng()}
            disabled={!current}
            aria-label="Save as PNG…"
            title={`Save the preview as a PNG image (${shortcut("S")})`}
          >
            <SaveIcon />
            as PNG
          </button>
        </div>
        <p className="preview-meta" aria-live="off">
          {rendering ? (
            <span className="rendering">Rendering…</span>
          ) : result ? (
            <>
              <strong>
                {result.width} × {result.height} dots · ≈ {formatMm(result.length_mm)}
              </strong>{" "}
              <span className="muted">
                {copies > 1 ? `× ${copies} copies, ` : ""}
                {settings && settings.feedMm > 0 ? `then ${settings.feedMm} mm extra feed` : "no extra feed"}
              </span>
            </>
          ) : (
            <span className="muted">&nbsp;</span>
          )}
        </p>
      </div>

      {/* Kept while a new preview renders, so the paper below does not jump on every edit. */}
      {preview.result && preview.result.warnings.length > 0 ? (
        <ul className="warnings" aria-label="Preview warnings">
          {preview.result.warnings.map((w) => (
            <li key={w}>
              <strong>Warning: </strong>
              {w}
            </li>
          ))}
        </ul>
      ) : null}

      <div className={`desk-scroll zoom-${zoom}`}>{body}</div>
    </section>
  );
}
