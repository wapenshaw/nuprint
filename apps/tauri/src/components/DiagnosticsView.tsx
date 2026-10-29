import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import { errorMessage } from "../lib/errors";
import { formatDateTime } from "../lib/format";
import { ADAPTER_MESSAGE, FLAG_LABEL } from "../lib/labels";
import { useApp } from "../state/AppStore";
import { ErrorNote } from "./ui";

export function DiagnosticsView() {
  const { data, notify, reportError } = useApp();
  const [includeIds, setIncludeIds] = useState(false);
  const [report, setReport] = useState<string | null>(null);
  const [reportFailure, setReportError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const info = data.info;
  const adapter = ADAPTER_MESSAGE[data.adapter];
  const check = data.lastCheck;

  async function exportReport() {
    setExporting(true);
    try {
      if (!(await api().exportDiagnostics(includeIds))) return;
      notify(
        includeIds
          ? "Exported the diagnostics report, including printer names and addresses."
          : "Exported the diagnostics report without printer names or addresses.",
        "success",
      );
    } catch (e) {
      reportError(e, "Could not export diagnostics.");
    } finally {
      setExporting(false);
    }
  }

  async function showReport() {
    setReportError(null);
    try {
      const r = await api().diagnosticsReport(includeIds);
      setReport(JSON.stringify(r, null, 2));
    } catch (e) {
      setReport(null);
      setReportError(errorMessage(e));
    }
  }

  return (
    <div className="settings-page">
      <div className="settings-head">
        <h2>Diagnostics</h2>
      </div>

      <section className="card" aria-labelledby="about-heading">
        <h3 id="about-heading">This app</h3>
        {info ? (
          <dl className="facts">
            <dt>Version</dt>
            <dd>{info.version}</dd>
            <dt>Renderer</dt>
            <dd>{info.renderer_version}</dd>
            <dt>System</dt>
            <dd>{info.os}</dd>
            <dt>Data folder</dt>
            <dd>{info.data_dir ?? "Not stored (settings and jobs are lost when the app closes)"}</dd>
            <dt>Saved between runs</dt>
            <dd>{info.persistent ? "Yes" : "No"}</dd>
            <dt>PDF support</dt>
            <dd>{info.pdf_available ? "Available" : `Not available${info.pdf_error ? `: ${info.pdf_error}` : ""}`}</dd>
            <dt>Bluetooth</dt>
            <dd>{adapter.label}</dd>
          </dl>
        ) : (
          <p className="muted">Loading…</p>
        )}
      </section>

      <section className="card" aria-labelledby="check-heading">
        <h3 id="check-heading">Last connection check</h3>
        {check ? (
          <div>
            <dl className="facts">
              <dt>Result</dt>
              <dd>
                <strong>{check.ready ? "Ready" : "Not ready"}</strong> · {check.status_text}
              </dd>
              <dt>When</dt>
              <dd>{formatDateTime(check.checked_ms)}</dd>
              <dt>Flags</dt>
              <dd>{check.flags.length ? check.flags.map((f) => FLAG_LABEL[f]).join(", ") : "None"}</dd>
              <dt>Raw status</dt>
              <dd className="mono">{check.raw_status_hex || "—"}</dd>
              <dt>Firmware</dt>
              <dd>{check.firmware ?? "Not reported"}</dd>
              <dt>Largest write</dt>
              <dd>{check.max_write_len} bytes</dd>
              <dt>Notifications</dt>
              <dd className="mono">{check.notifications.length ? check.notifications.join(", ") : "None"}</dd>
              <dt>Services</dt>
              <dd className="mono">
                {check.services.map((s) => (
                  <div key={s.uuid}>
                    {s.uuid}
                    {s.characteristics.map((c) => (
                      <div key={c.uuid} className="indent">
                        {c.uuid} ({c.properties.join(", ")})
                      </div>
                    ))}
                  </div>
                ))}
              </dd>
            </dl>
          </div>
        ) : (
          <p className="muted">No check yet. Use “Check connection” in the printer menu or in Printers.</p>
        )}
      </section>

      <section className="card" aria-labelledby="log-heading">
        <h3 id="log-heading">Activity log</h3>
        <LogView />
      </section>

      <section className="card" aria-labelledby="export-heading">
        <h3 id="export-heading">Share a report</h3>
        <p className="muted">
          A report lists the app version, printer models, recent job outcomes and the log. Printer names and
          addresses are left out unless you include them.
        </p>
        <label className="check">
          <input type="checkbox" checked={includeIds} onChange={(e) => setIncludeIds(e.target.checked)} />
          Include printer names and addresses
        </label>
        <div className="row">
          <button type="button" className="btn btn-primary" onClick={() => void exportReport()} disabled={exporting}>
            Export…
          </button>
          <button type="button" className="btn" onClick={() => void showReport()}>
            Preview report
          </button>
        </div>
        {reportFailure ? <ErrorNote>{reportFailure}</ErrorNote> : null}
        {report ? (
          <pre className="report" aria-label="Diagnostics report preview" tabIndex={0}>
            {report}
          </pre>
        ) : null}
      </section>
    </div>
  );
}

const LEVEL_LABEL = { info: "INFO", warn: "WARN", error: "ERROR", debug: "DEBUG" } as const;

function LogView() {
  const { data } = useApp();
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const lines = data.logs;

  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div
      ref={ref}
      className="log"
      role="log"
      aria-label="Activity log"
      tabIndex={0}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
    >
      {lines.length === 0 ? <p className="muted">Nothing logged yet.</p> : null}
      {lines.map((l) => (
        <div key={l.seq} className={`log-line log-${l.level}`}>
          <span className="log-time">{new Date(l.at_ms).toLocaleTimeString()}</span>{" "}
          <span className="log-level">{LEVEL_LABEL[l.level]}</span> {l.message}
        </div>
      ))}
    </div>
  );
}
