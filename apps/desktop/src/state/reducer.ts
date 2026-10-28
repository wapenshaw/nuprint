import type { AdapterStatus } from "../bindings/AdapterStatus";
import type { AppError } from "../bindings/AppError";
import type { AppEvent } from "../bindings/AppEvent";
import type { AppInfo } from "../bindings/AppInfo";
import type { DiscoveredPrinter } from "../bindings/DiscoveredPrinter";
import type { JobSummary } from "../bindings/JobSummary";
import type { LogLine } from "../bindings/LogLine";
import type { PrinterCheck } from "../bindings/PrinterCheck";
import type { ProfileSummary } from "../bindings/ProfileSummary";
import type { SavedPrinter } from "../bindings/SavedPrinter";

export const MAX_LOG_LINES = 500;

export type StatusTone = "info" | "success" | "error";

export interface StatusMessage {
  id: number;
  tone: StatusTone;
  text: string;
  /** Job whose live state and progress the status bar shows beneath the message. */
  jobId?: string;
  action?: { label: string; target: "queue" | "printers" | "workspace" };
}

export interface ScanState {
  status: "idle" | "starting" | "scanning" | "finished";
  scanId: string | null;
  devices: DiscoveredPrinter[];
  found: number;
  error: AppError | null;
  /** Events that arrived before start_scan returned its scan id. */
  early: AppEvent[];
}

export interface AppData {
  ready: boolean;
  loadError: AppError | null;
  info: AppInfo | null;
  profiles: ProfileSummary[];
  printers: SavedPrinter[];
  selectedPrinterId: string | null;
  jobs: JobSummary[];
  queueHeld: boolean;
  progress: Record<string, { sent: number; total: number }>;
  adapter: AdapterStatus;
  logs: LogLine[];
  checks: Record<string, PrinterCheck>;
  lastCheck: PrinterCheck | null;
  scan: ScanState;
  status: StatusMessage | null;
}

export const initialScan: ScanState = { status: "idle", scanId: null, devices: [], found: 0, error: null, early: [] };

export const initialData: AppData = {
  ready: false,
  loadError: null,
  info: null,
  profiles: [],
  printers: [],
  selectedPrinterId: null,
  jobs: [],
  queueHeld: false,
  progress: {},
  adapter: "unknown",
  logs: [],
  checks: {},
  lastCheck: null,
  scan: initialScan,
  status: null,
};

export type Action =
  | {
      type: "loaded";
      info: AppInfo;
      profiles: ProfileSummary[];
      printers: SavedPrinter[];
      selectedPrinterId: string | null;
      jobs: JobSummary[];
      queueHeld: boolean;
      logs: LogLine[];
      adapter: AdapterStatus;
    }
  | { type: "load_failed"; error: AppError }
  | { type: "event"; event: AppEvent }
  | { type: "printers"; printers: SavedPrinter[]; profiles?: ProfileSummary[]; selectedPrinterId?: string | null }
  | { type: "printer_updated"; printer: SavedPrinter }
  | { type: "selected"; id: string | null }
  | { type: "queue"; jobs: JobSummary[]; held: boolean }
  | { type: "job_upsert"; job: JobSummary }
  | { type: "job_remove"; id: string }
  | { type: "adapter"; adapter: AdapterStatus }
  | { type: "check"; check: PrinterCheck }
  | { type: "scan_starting" }
  | { type: "scan_started"; scanId: string; adapter: AdapterStatus }
  | { type: "scan_failed"; error: AppError }
  | { type: "scan_stopped" }
  | { type: "status"; message: StatusMessage | null };

/** Newest first. */
function sortJobs(jobs: JobSummary[]): JobSummary[] {
  return [...jobs].sort((a, b) => b.created_ms - a.created_ms || b.updated_ms - a.updated_ms);
}

/** Replace by id only when the incoming revision is not older than what we hold. */
export function upsertJob(jobs: JobSummary[], job: JobSummary): JobSummary[] {
  const i = jobs.findIndex((j) => j.id === job.id);
  if (i < 0) return sortJobs([...jobs, job]);
  const cur = jobs[i]!;
  if (job.revision < cur.revision) return jobs;
  const next = [...jobs];
  next[i] = job;
  return sortJobs(next);
}

function upsertDevice(devices: DiscoveredPrinter[], device: DiscoveredPrinter): DiscoveredPrinter[] {
  const i = devices.findIndex((d) => d.discovery_id === device.discovery_id);
  if (i < 0) return [...devices, device];
  const next = [...devices];
  next[i] = device;
  return next;
}

function applyScanEvent(scan: ScanState, event: AppEvent): ScanState {
  if (event.type !== "scan_device" && event.type !== "scan_finished") return scan;
  if (scan.status === "starting") return { ...scan, early: [...scan.early, event] };
  if (event.scan_id !== scan.scanId) return scan;
  if (event.type === "scan_device") {
    return { ...scan, devices: upsertDevice(scan.devices, event.device) };
  }
  return { ...scan, status: "finished", found: event.found, error: event.error ?? null };
}

function appendLog(logs: LogLine[], line: LogLine): LogLine[] {
  if (logs.some((l) => l.seq === line.seq)) return logs;
  const next = [...logs, line];
  return next.length > MAX_LOG_LINES ? next.slice(next.length - MAX_LOG_LINES) : next;
}

function applyEvent(state: AppData, event: AppEvent): AppData {
  switch (event.type) {
    case "scan_device":
    case "scan_finished":
      return { ...state, scan: applyScanEvent(state.scan, event) };
    case "job_changed": {
      const jobs = upsertJob(state.jobs, event.job);
      return { ...state, jobs, queueHeld: event.queue_held };
    }
    case "job_removed": {
      const { [event.job_id]: _drop, ...progress } = state.progress;
      return { ...state, jobs: state.jobs.filter((j) => j.id !== event.job_id), progress };
    }
    case "job_progress":
      return {
        ...state,
        progress: { ...state.progress, [event.job_id]: { sent: event.bytes_sent, total: event.bytes_total } },
      };
    case "log":
      return { ...state, logs: appendLog(state.logs, event.line) };
    case "printers_changed":
      // Handled by the provider, which re-reads durable state.
      return state;
  }
}

export function reducer(state: AppData, action: Action): AppData {
  switch (action.type) {
    case "loaded":
      return {
        ...state,
        ready: true,
        loadError: null,
        info: action.info,
        profiles: action.profiles,
        printers: action.printers,
        selectedPrinterId: action.selectedPrinterId,
        jobs: sortJobs(action.jobs),
        queueHeld: action.queueHeld,
        logs: action.logs.slice(-MAX_LOG_LINES),
        adapter: action.adapter,
      };
    case "load_failed":
      return { ...state, ready: true, loadError: action.error };
    case "event":
      return applyEvent(state, action.event);
    case "printers":
      return {
        ...state,
        printers: action.printers,
        profiles: action.profiles ?? state.profiles,
        selectedPrinterId: action.selectedPrinterId === undefined ? state.selectedPrinterId : action.selectedPrinterId,
      };
    case "printer_updated":
      return { ...state, printers: state.printers.map((p) => (p.id === action.printer.id ? action.printer : p)) };
    case "selected":
      return { ...state, selectedPrinterId: action.id };
    case "queue":
      return { ...state, jobs: sortJobs(action.jobs), queueHeld: action.held };
    case "job_upsert":
      return { ...state, jobs: upsertJob(state.jobs, action.job) };
    case "job_remove":
      return { ...state, jobs: state.jobs.filter((j) => j.id !== action.id) };
    case "adapter":
      return { ...state, adapter: action.adapter };
    case "check":
      return {
        ...state,
        checks: { ...state.checks, [action.check.printer_id]: action.check },
        lastCheck: action.check,
      };
    case "scan_starting":
      return { ...state, scan: { ...initialScan, status: "starting" } };
    case "scan_started": {
      let scan: ScanState = { ...state.scan, status: "scanning", scanId: action.scanId };
      const early = scan.early;
      scan = { ...scan, early: [] };
      for (const e of early) scan = applyScanEvent(scan, e);
      // The adapter can refuse to scan; the finished state then comes from the caller.
      return { ...state, adapter: action.adapter, scan };
    }
    case "scan_failed":
      return { ...state, scan: { ...state.scan, status: "finished", error: action.error, early: [] } };
    case "scan_stopped":
      return { ...state, scan: { ...state.scan, status: "finished" } };
    case "status":
      return { ...state, status: action.message };
  }
}
