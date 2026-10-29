import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type { DeviceInspection } from "../bindings/DeviceInspection";
import type { DiscoveredPrinter } from "../bindings/DiscoveredPrinter";
import type { PrinterCheck } from "../bindings/PrinterCheck";
import type { ProfileMatch } from "../bindings/ProfileMatch";
import type { ProfileSummary } from "../bindings/ProfileSummary";
import type { SavedPrinter } from "../bindings/SavedPrinter";
import { isAppError, toAppError } from "../lib/errors";
import { formatDateTime, formatRssi } from "../lib/format";
import { ADAPTER_MESSAGE, FLAG_LABEL, SUPPORT_LABEL } from "../lib/labels";
import { useApp } from "../state/AppStore";
import { PrinterAdvanced } from "./PrinterAdvanced";
import { Badge, ErrorNote, Modal, SupportBadge, WarnNote } from "./ui";
import { PrinterLight } from "./PrinterLight";

export function PrintersView() {
  return (
    <div className="settings-page">
      <div className="settings-head">
        <h2>Printers</h2>
      </div>
      <SavedPrinters />
      <FindPrinters />
      <p className="hint">
        Only one device can talk to the printer at a time. If a printer is not found or will not connect, close the
        printer's phone app (or turn off the phone's Bluetooth) and try again. NuPrint does not need Bluetooth pairing for
        supported printers.
      </p>
    </div>
  );
}

/* ------------------------------------------------------------------ saved printers */

function SavedPrinters() {
  const { data, selectPrinter, refreshPrinters, reportError, notify, checking, checkErrors, runCheck } = useApp();
  const [removing, setRemoving] = useState<SavedPrinter | null>(null);
  const [advanced, setAdvanced] = useState<SavedPrinter | null>(null);

  return (
    <section className="card" aria-labelledby="saved-heading">
      <h3 id="saved-heading">Saved printers</h3>
      {data.printers.length === 0 ? (
        <p className="muted">No printers saved yet. Use “Find printers” below to add one.</p>
      ) : (
        <ul className="printer-list">
          {data.printers.map((p) => {
            const isDefault = p.id === data.selectedPrinterId;
            const check = data.checks[p.id];
            return (
              <li key={p.id} className="printer">
                <PrinterLight name={p.name} check={check} checking={checking === p.id} failed={Boolean(checkErrors[p.id])} />
                <div className="printer-main">
                  <div className="job-title-row">
                    <span className="job-title">{p.name}</span>
                    <SupportBadge level={p.support} />
                    {p.customized ? <Badge tone="warn" title="Settings edited on this computer">Edited settings</Badge> : null}
                    {isDefault ? <Badge tone="active">Default</Badge> : null}
                  </div>
                  <p className="job-meta">
                    {p.profile_name}
                    {p.firmware ? ` · firmware ${p.firmware}` : ""}
                    {p.last_seen_ms ? (
                      <>
                        {" · last seen "}
                        <span className="nowrap">{formatDateTime(p.last_seen_ms)}</span>
                      </>
                    ) : (
                      " · not seen yet"
                    )}
                  </p>
                  {!p.profile_available ? (
                    <WarnNote>
                      The saved profile for this printer is not available in this version of NuPrint, so it cannot
                      print until the app is updated.
                    </WarnNote>
                  ) : null}
                  {checking === p.id ? <p role="status">Checking the connection… This does not print anything.</p> : null}
                  {checkErrors[p.id] ? <ErrorNote>{checkErrors[p.id]}</ErrorNote> : null}
                  {check && checking !== p.id ? <CheckResult check={check} /> : null}
                </div>
                <div className="job-actions">
                  <button
                    type="button"
                    className="btn btn-small"
                    onClick={() => void runCheck(p.id)}
                    disabled={checking !== null}
                    aria-label={`Check connection for ${p.name}`}
                  >
                    Check connection
                  </button>
                  {!isDefault ? (
                    <button
                      type="button"
                      className="btn btn-small"
                      onClick={() => void selectPrinter(p.id)}
                      aria-label={`Make ${p.name} the default printer`}
                    >
                      Make default
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="btn btn-quiet btn-small"
                    onClick={() => setAdvanced(p)}
                    aria-label={`Advanced settings for ${p.name}`}
                  >
                    Advanced
                  </button>
                  <button
                    type="button"
                    className="btn btn-quiet btn-small"
                    onClick={() => setRemoving(p)}
                    aria-label={`Remove ${p.name}`}
                  >
                    Remove
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {advanced ? <PrinterAdvanced printer={advanced} onClose={() => setAdvanced(null)} /> : null}

      {removing ? (
        <Modal
          title={`Remove ${removing.name}?`}
          onClose={() => setRemoving(null)}
          footer={
            <>
              <button type="button" className="btn" onClick={() => setRemoving(null)}>
                Keep printer
              </button>
              <button
                type="button"
                className="btn btn-danger"
                onClick={async () => {
                  const target = removing;
                  setRemoving(null);
                  try {
                    await api().removePrinter(target.id);
                    await refreshPrinters();
                    notify(`Removed ${target.name}.`, "success");
                  } catch (e) {
                    reportError(e, "Could not remove the printer.");
                  }
                }}
              >
                Remove printer
              </button>
            </>
          }
        >
          <p>
            The printer is forgotten by NuPrint. Jobs already in the queue keep their original printer details. You
            can add it again at any time.
          </p>
        </Modal>
      ) : null}
    </section>
  );
}

function CheckResult({ check }: { check: PrinterCheck }) {
  return (
    <div className={`check-result ${check.ready ? "check-ok" : "check-bad"}`} role="status">
      <p>
        <strong>{check.ready ? "Ready" : "Not ready"}</strong> · {check.status_text}
        <span className="muted"> · checked {formatDateTime(check.checked_ms)}</span>
      </p>
      {check.flags.length > 0 ? (
        <p>
          Status: {check.flags.map((f) => FLAG_LABEL[f]).join(", ")}
        </p>
      ) : null}
      {check.firmware ? <p className="muted">Firmware {check.firmware}</p> : null}
    </div>
  );
}

/* ------------------------------------------------------------------ discovery */

function FindPrinters() {
  const { data, startScan, stopScan, checkAdapter, reportError } = useApp();
  const [includeUnknown, setIncludeUnknown] = useState(false);
  const [inspecting, setInspecting] = useState<DiscoveredPrinter | null>(null);
  const [saving, setSaving] = useState<DiscoveredPrinter | null>(null);
  const scan = data.scan;
  const scanning = scan.status === "starting" || scan.status === "scanning";
  const adapter = ADAPTER_MESSAGE[data.adapter];
  const asking = data.adapter === "permission_not_asked" || data.adapter === "permission_pending";
  // Only macOS grants Bluetooth per app, so only there is a settings page worth opening.
  const needsAccess =
    data.info?.os === "macos" &&
    (data.adapter === "permission_denied" ||
      data.adapter === "permission_pending" ||
      scan.error?.code === "permission_denied");

  // Refresh the adapter state when the screen opens, after each scan (which may have shown the
  // macOS permission prompt) and on returning to the window, e.g. from System Settings.
  useEffect(() => {
    void checkAdapter();
  }, [checkAdapter]);
  useEffect(() => {
    if (scan.status === "finished") void checkAdapter();
  }, [scan.status, checkAdapter]);
  useEffect(() => {
    const onFocus = () => void checkAdapter();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [checkAdapter]);

  // Supported printers first, otherwise in the order they were found. Not by signal: readings
  // change during a scan and would reshuffle the rows under the pointer.
  const devices = useMemo(
    () => [...scan.devices].sort((a, b) => Number(b.candidates.length > 0) - Number(a.candidates.length > 0)),
    [scan.devices],
  );

  const scanNote = scanning
    ? asking
      ? "If macOS asks whether NuPrint may use Bluetooth, choose Allow. The scan continues once you answer."
      : "Scanning for about 8 seconds…"
    : scan.status === "finished" && !scan.error
      ? devices.length > 0
        ? `Scan finished. ${devices.length} ${devices.length === 1 ? "device" : "devices"} found.`
        : data.adapter === "ready"
          ? "Scan finished. No printers found. Turn the printer on, keep it close, and close its phone app."
          : ""
      : "";

  return (
    <section className="card" aria-labelledby="find-heading">
      <h3 id="find-heading">Find printers</h3>

      {/* The scan's progress and result take the Bluetooth line's place, so the card keeps its height. */}
      <p className={`adapter adapter-${adapter.tone}`} role="status" data-testid="adapter-status">
        <strong>{adapter.label}. </strong>
        {scanNote || adapter.text}
      </p>
      {needsAccess ? (
        <div className="row">
          <button
            type="button"
            className="btn btn-small"
            onClick={() => api().openBluetoothSettings().catch((e) => reportError(e, "Could not open settings."))}
          >
            Open Bluetooth settings
          </button>
          <span className="muted">
            Turn on NuPrint there, then find printers again. If it still can't, quit and reopen NuPrint.
          </span>
        </div>
      ) : null}

      <div className="row">
        {scanning ? (
          <button type="button" className="btn" onClick={() => void stopScan()}>
            Stop scan
          </button>
        ) : (
          <button type="button" className="btn btn-primary" onClick={() => void startScan(includeUnknown)}>
            Find printers
          </button>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={includeUnknown}
            disabled={scanning}
            onChange={(e) => setIncludeUnknown(e.target.checked)}
          />
          Show unknown devices
        </label>
        <button type="button" className="btn btn-quiet btn-small" onClick={() => void checkAdapter()}>
          Recheck Bluetooth
        </button>
      </div>

      {scan.error ? <ErrorNote>{scan.error.message}</ErrorNote> : null}

      {devices.length > 0 ? (
        <ul className="device-list" aria-label="Devices found">
          {devices.map((d) => (
            <li key={d.discovery_id} className="device">
              <div className="printer-main">
                <div className="job-title-row">
                  <span className="job-title">{d.name ?? "Unnamed device"}</span>
                  {d.candidates.length === 0 ? <Badge tone="neutral">Unknown device</Badge> : null}
                  {d.candidates.map((c) => (
                    <SupportBadge key={c.profile_id} level={c.support} />
                  ))}
                  {d.saved_printer_id ? <Badge tone="active">Saved</Badge> : null}
                </div>
                <p className="job-meta">
                  {formatRssi(d.rssi)}
                  {d.address ? ` · ${d.address}` : ""}
                  {d.candidates.length > 0 ? ` · ${d.candidates.map((c) => c.display_name).join(" or ")}` : ""}
                </p>
              </div>
              <div className="job-actions">
                <button
                  type="button"
                  className="btn btn-small"
                  onClick={() => setInspecting(d)}
                  aria-label={`Inspect ${d.name ?? "unnamed device"}`}
                >
                  Inspect
                </button>
                <button
                  type="button"
                  className="btn btn-primary btn-small"
                  onClick={() => setSaving(d)}
                  disabled={d.candidates.length === 0}
                  aria-label={`Save ${d.name ?? "unnamed device"}`}
                  title={d.candidates.length === 0 ? "No printer profile matches this device" : undefined}
                >
                  Save…
                </button>
              </div>
            </li>
          ))}
        </ul>
      ) : null}

      {inspecting ? <InspectDialog device={inspecting} onClose={() => setInspecting(null)} /> : null}
      {saving ? <SaveDialog device={saving} onClose={() => setSaving(null)} /> : null}
    </section>
  );
}

function InspectDialog({ device, onClose }: { device: DiscoveredPrinter; onClose: () => void }) {
  const [state, setState] = useState<
    { status: "loading" } | { status: "error"; message: string } | { status: "done"; value: DeviceInspection }
  >({ status: "loading" });
  // One inspection per device, even when development mode runs the effect twice: a second
  // request would only wait for the first to release Bluetooth.
  const request = useRef<{ id: string; result: Promise<DeviceInspection> } | null>(null);

  useEffect(() => {
    if (request.current?.id !== device.discovery_id) {
      request.current = { id: device.discovery_id, result: api().inspectDevice(device.discovery_id) };
    }
    let cancelled = false;
    request.current.result
      .then((value) => !cancelled && setState({ status: "done", value }))
      .catch((e) => !cancelled && setState({ status: "error", message: toAppError(e).message }));
    return () => {
      cancelled = true;
    };
  }, [device.discovery_id]);

  // Closing while it connects stops the inspection, so Bluetooth is free for the next action.
  const close = () => {
    if (state.status === "loading") api().cancelInspect().catch(() => undefined);
    onClose();
  };

  return (
    <Modal title={`Inspect ${device.name ?? "device"}`} onClose={close} wide>
      {state.status === "loading" ? (
        <p role="status">Connecting briefly to read the device's services… Nothing is printed or changed.</p>
      ) : null}
      {state.status === "error" ? <ErrorNote>{state.message}</ErrorNote> : null}
      {state.status === "done" ? (
        <div className="inspect">
          <h3>Compatible printer profiles</h3>
          {state.value.compatible.length === 0 ? (
            <p className="muted">None. This device does not match any supported printer.</p>
          ) : (
            <ul>
              {state.value.compatible.map((c) => (
                <li key={c.profile_id}>
                  {c.display_name} <SupportBadge level={c.support} />
                </li>
              ))}
            </ul>
          )}
          {state.value.rejected.length > 0 ? (
            <>
              <h3>Rejected</h3>
              <ul>
                {state.value.rejected.map((r) => (
                  <li key={r}>{r}</li>
                ))}
              </ul>
            </>
          ) : null}
          <h3>Services</h3>
          <p className="muted">Largest write the device accepts: {state.value.max_write_len} bytes.</p>
          {state.value.services.length === 0 ? (
            <p className="muted">No services were reported.</p>
          ) : (
            <ul className="mono">
              {state.value.services.map((s) => (
                <li key={s.uuid}>
                  {s.uuid}
                  <ul>
                    {s.characteristics.map((c) => (
                      <li key={c.uuid}>
                        {c.uuid} <span className="muted">({c.properties.join(", ")})</span>
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </Modal>
  );
}

function SaveDialog({ device, onClose }: { device: DiscoveredPrinter; onClose: () => void }) {
  const { data, refreshPrinters, notify } = useApp();
  const candidates = device.candidates;
  const savable = candidates.filter((c) => c.support !== "candidate");
  const [profileId, setProfileId] = useState<string>(savable[0]?.profile_id ?? candidates[0]?.profile_id ?? "");
  const [ack, setAck] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const chosen: ProfileMatch | undefined = candidates.find((c) => c.profile_id === profileId);
  const profile: ProfileSummary | undefined = data.profiles.find((p) => p.id === profileId);
  const support = chosen?.support ?? "candidate";
  const experimental = support === "experimental";
  const canSave = Boolean(chosen) && support !== "candidate" && (!experimental || ack) && !busy;

  async function save() {
    if (!chosen) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await api().savePrinter({
        discovery_id: device.discovery_id,
        profile_id: chosen.profile_id,
        acknowledge_experimental: experimental && ack,
      });
      await refreshPrinters();
      notify(`Saved ${saved.name}. You can check the connection or print now.`, "success", {
        label: "Close settings",
        target: "workspace",
      });
      onClose();
    } catch (e) {
      const ae = toAppError(e);
      setError(
        isAppError(e) && ae.code === "experimental_not_acknowledged"
          ? "Tick the box to confirm you understand this printer is experimental."
          : ae.message,
      );
      setBusy(false);
    }
  }

  return (
    <Modal
      title={`Save ${device.name ?? "printer"}`}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void save()} disabled={!canSave}>
            {busy ? "Saving…" : "Save printer"}
          </button>
        </>
      }
    >
      <fieldset className="profile-choice">
        <legend className="field-label">Printer model</legend>
        {candidates.map((c) => (
          <label key={c.profile_id} className="check">
            <input
              type="radio"
              name="profile"
              value={c.profile_id}
              checked={profileId === c.profile_id}
              onChange={() => {
                setProfileId(c.profile_id);
                setAck(false);
              }}
            />
            {c.display_name} <Badge tone={c.support === "candidate" ? "bad" : c.support === "experimental" ? "warn" : "good"}>{SUPPORT_LABEL[c.support]}</Badge>
          </label>
        ))}
      </fieldset>

      {support === "candidate" ? (
        <ErrorNote>This model is recognised by name but is not supported yet, so it cannot be saved.</ErrorNote>
      ) : null}

      {profile?.support_note ? <p className="support-note">{profile.support_note}</p> : null}

      {experimental ? (
        <>
          <WarnNote>
            Experimental printers are not verified on this operating system. Printing may fail, come out too light or
            dark, or waste paper.
          </WarnNote>
          <label className="check">
            <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />I understand this printer
            is experimental and I want to use it anyway
          </label>
        </>
      ) : null}
      {error ? <ErrorNote>{error}</ErrorNote> : null}
    </Modal>
  );
}
