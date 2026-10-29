/**
 * In-memory fake backend. Used automatically when the page is not running inside Tauri
 * (`npm run dev` in a browser, vitest). It imitates the documented command contract closely
 * enough to exercise every screen: scanning, saving (with the experimental acknowledgement),
 * preview generations (with stale rejection), idempotent enqueue and job state progression.
 * It never produces real raster data: previews are SVG data URLs.
 */
import type { AdapterStatus } from "../bindings/AdapterStatus";
import type { AppError } from "../bindings/AppError";
import type { AppEvent } from "../bindings/AppEvent";
import type { AppInfo } from "../bindings/AppInfo";
import type { DeviceInspection } from "../bindings/DeviceInspection";
import type { DiagnosticsReport } from "../bindings/DiagnosticsReport";
import type { DiscoveredPrinter } from "../bindings/DiscoveredPrinter";
import type { EnqueueRequest } from "../bindings/EnqueueRequest";
import type { JobState } from "../bindings/JobState";
import type { JobSummary } from "../bindings/JobSummary";
import type { LogLevel } from "../bindings/LogLevel";
import type { LogLine } from "../bindings/LogLine";
import type { PreviewRequest } from "../bindings/PreviewRequest";
import type { PreviewResult } from "../bindings/PreviewResult";
import type { PrinterCheck } from "../bindings/PrinterCheck";
import type { ProfileMatch } from "../bindings/ProfileMatch";
import type { ProfileSummary } from "../bindings/ProfileSummary";
import type { QueueSnapshot } from "../bindings/QueueSnapshot";
import type { SavedPrinter } from "../bindings/SavedPrinter";
import type { SourceInfo } from "../bindings/SourceInfo";
import type { UpdateInfo } from "../bindings/UpdateInfo";
import { isActive } from "../lib/labels";
import type { Backend } from "./types";

/** A newer release, as a check finds it (`?mock=update` in the browser). */
export const SAMPLE_UPDATE: UpdateInfo = {
  version: "1.2.0",
  current_version: "1.0.0-mock",
  notes: "Features\n• Check for updates and install them from Settings\n\nBug fixes\n• Keep log lines that arrive out of order",
  release_url: "https://github.com/wapenshaw/nuprint/releases/tag/v1.2.0",
  can_install: true,
};

export interface MockOptions {
  /** Start with one saved, selected printer (default true). */
  seedPrinters?: boolean;
  /** Latency added to every command (default 20 ms). */
  latencyMs?: number;
  /** Per-call latency for render_preview (call index starts at 1). Overrides latencyMs. */
  previewLatencyMs?: (callIndex: number) => number;
  /** Move jobs through connecting → sending → ready on timers (default true). */
  autoAdvance?: boolean;
  /** Delay between job state steps (default 700 ms). */
  stepMs?: number;
  /** Real duration of a scan regardless of the requested duration (default 2500 ms). */
  scanMs?: number;
  adapter?: AdapterStatus;
  /** Operating system reported by get_app_info (default "Mock browser"). */
  os?: string;
  /** Path returned by the open dialog (null simulates cancel). */
  openPath?: string | null;
  /** Path returned by the save dialog (null simulates cancel). */
  savePath?: string | null;
  /** What a check for updates finds (default: nothing newer). */
  update?: UpdateInfo | null;
}

export interface MockBackend extends Backend {
  /** Recorded calls, for assertions. */
  readonly calls: {
    enqueue: EnqueueRequest[];
    renderPreview: PreviewRequest[];
    updateSettings: { printerId: string; density: string; feed_mm: number }[];
    savePrinter: { discovery_id: string; profile_id: string; acknowledge_experimental: boolean }[];
    savePrinterProfile: { printer_id: string; json: string }[];
    resetPrinterProfile: string[];
    exportPrinterProfile: string[];
    inspect: string[];
    cancelInspect: number;
    openBluetoothSettings: number;
    setAppIcon: string[];
    setWindowTheme: ("light" | "dark" | null)[];
    cancel: string[];
    savePng: { snapshotId: string; path: string }[];
    exportDiagnostics: { includeIdentifiers: boolean }[];
    checkForUpdate: number;
    installUpdate: number;
    openReleasePage: number;
  };
  readonly options: Required<Omit<MockOptions, "previewLatencyMs">> & Pick<MockOptions, "previewLatencyMs">;
  /** Insert a job directly into the queue and notify subscribers. */
  seedJob(partial: Partial<JobSummary> & { state: JobState }): JobSummary;
  emit(event: AppEvent): void;
  /** Errors consumed one per render_preview call, before normal handling. */
  previewErrors: AppError[];
}

export const MOCK_PROFILES: ProfileSummary[] = [
  {
    id: "sc03h",
    revision: 1,
    display_name: "SC03h",
    vendor: "Generic",
    model: "SC03h",
    protocol_id: "sc03h-ble",
    support: "verified_basic",
    support_note: "Verified on one unit over Bluetooth LE on this operating system.",
    width_dots: 384,
    dots_per_mm: 8,
    max_height: 4000,
    densities: ["light", "normal", "dark"],
    default_density: "normal",
    feed: { min_mm: 0, max_mm: 12, default_mm: 12, calibrated_reduced: false },
    max_copies: 20,
    media: "58 mm continuous roll (48 mm printable)",
    completion: "status_confirmed_ready",
  },
  {
    id: "demo-wide",
    revision: 1,
    display_name: "Demo Wide 80",
    vendor: "Example",
    model: "DW-80",
    protocol_id: "demo-ble",
    support: "experimental",
    support_note:
      "Experimental: this model has not been tested on your operating system. Output may be misaligned, too light or fail partway.",
    width_dots: 576,
    dots_per_mm: 8,
    max_height: 4000,
    densities: ["normal", "dark"],
    default_density: "normal",
    feed: { min_mm: 0, max_mm: 8, default_mm: 4, calibrated_reduced: true },
    max_copies: 10,
    media: "80 mm continuous roll (72 mm printable)",
    completion: "unconfirmed_transfer",
  },
];

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

const err = (code: AppError["code"], message: string, detail?: string): AppError =>
  detail ? { code, message, detail } : { code, message };

const match = (p: ProfileSummary): ProfileMatch => ({
  profile_id: p.id,
  revision: p.revision,
  display_name: p.display_name,
  support: p.support,
});

function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function svgDataUrl(width: number, height: number, body: string): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" shape-rendering="crispEdges">` +
    `<rect width="100%" height="100%" fill="#fff"/>${body}</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

export function createMockBackend(opts: MockOptions = {}): MockBackend {
  const options = {
    seedPrinters: opts.seedPrinters ?? true,
    latencyMs: opts.latencyMs ?? 20,
    autoAdvance: opts.autoAdvance ?? true,
    stepMs: opts.stepMs ?? 700,
    scanMs: opts.scanMs ?? 2500,
    adapter: opts.adapter ?? ("ready" as AdapterStatus),
    os: opts.os ?? "Mock browser",
    openPath: opts.openPath === undefined ? "C:\\Users\\you\\Pictures\\sample-photo.png" : opts.openPath,
    savePath: opts.savePath === undefined ? "C:\\Users\\you\\Documents\\nuprint-preview.png" : opts.savePath,
    previewLatencyMs: opts.previewLatencyMs,
    update: opts.update ?? null,
  };

  const handlers = new Set<(e: AppEvent) => void>();
  let seq = 0;
  let previewCalls = 0;
  let maxGeneration = 0;
  let nextId = 1;
  const newId = (prefix: string) => `${prefix}-${nextId++}`;
  let lastTs = 0;
  /** Strictly increasing millisecond timestamps so job ordering is deterministic. */
  const tick = () => (lastTs = Math.max(Date.now(), lastTs + 1));

  const calls: MockBackend["calls"] = {
    enqueue: [],
    renderPreview: [],
    updateSettings: [],
    savePrinter: [],
    savePrinterProfile: [],
    resetPrinterProfile: [],
    exportPrinterProfile: [],
    inspect: [],
    cancelInspect: 0,
    openBluetoothSettings: 0,
    setAppIcon: [],
    setWindowTheme: [],
    cancel: [],
    savePng: [],
    exportDiagnostics: [],
    checkForUpdate: 0,
    installUpdate: 0,
    openReleasePage: 0,
  };

  let printers: SavedPrinter[] = [];
  let selected: string | null = null;
  let jobs: JobSummary[] = [];
  let held = false;
  let queueRevision = 1;
  const idempotent = new Map<string, JobSummary>();
  const logs: LogLine[] = [];
  const sources = new Map<string, { info: SourceInfo; locked: boolean }>();
  const discovered = new Map<string, DiscoveredPrinter>();
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let scanId: string | null = null;
  let lastCheck: PrinterCheck | undefined;

  const infoApp: AppInfo = {
    version: "1.0.0-mock",
    renderer_version: "mock-renderer 1",
    os: options.os,
    data_dir: null,
    pdf_available: true,
    persistent: false,
  };

  /** Profiles created by editing a printer's settings, as the real backend lists them. */
  const customProfiles: ProfileSummary[] = [];
  const profileById = (id: string) => [...MOCK_PROFILES, ...customProfiles].find((p) => p.id === id);
  const printerDocs = new Map<string, { base: string; current: string }>();
  /** Built-in profile each edited printer came from. */
  const printerBases = new Map<string, string>();
  const baseDoc = (p: ProfileSummary) =>
    JSON.stringify(
      {
        display_name: p.display_name,
        transport: { max_chunk_bytes: 180, pacing: { chunk_delay_ms: 25, boundary_bytes: 4096, boundary_delay_ms: 250 } },
        raster: { width_dots: p.width_dots, dots_per_mm: p.dots_per_mm, max_height: p.max_height },
        limits: { max_copies: p.max_copies },
        driver: { energy: { light: 4250, normal: 5000, dark: 5750 }, print_speed: 10 },
        default_density: p.default_density,
      },
      null,
      2,
    );
  const LOCKED_KEYS = ["id", "protocol", "match", "vendor", "model", "verification", "evidence"];

  function emit(event: AppEvent) {
    for (const h of [...handlers]) h(event);
  }

  function log(level: LogLevel, message: string) {
    const line: LogLine = { seq: ++seq, at_ms: Date.now(), level, message };
    logs.push(line);
    if (logs.length > 500) logs.shift();
    emit({ type: "log", line });
  }

  function later(ms: number, fn: () => void) {
    const t = setTimeout(() => {
      timers.delete(t);
      fn();
    }, ms);
    timers.add(t);
    return t;
  }

  function sortedJobs(): JobSummary[] {
    return [...jobs].sort((a, b) => b.created_ms - a.created_ms);
  }

  function snapshot(): QueueSnapshot {
    return { held, revision: queueRevision, seq, jobs: sortedJobs() };
  }

  function putJob(job: JobSummary) {
    const i = jobs.findIndex((j) => j.id === job.id);
    if (i >= 0) jobs[i] = job;
    else jobs.push(job);
    queueRevision++;
    emit({ type: "job_changed", seq: ++seq, job, queue_held: held });
  }

  function patchJob(id: string, patch: Partial<JobSummary>) {
    const cur = jobs.find((j) => j.id === id);
    if (!cur) return;
    putJob({ ...cur, ...patch, updated_ms: Date.now(), revision: cur.revision + 1 });
  }

  const STEP_TEXT: Partial<Record<JobState, string>> = {
    held: "Held until you resume the queue",
    queued: "Waiting for the printer",
    connecting: "Connecting to the printer…",
    sending: "Sending data to the printer…",
    waiting_for_ready: "Waiting for the printer to report ready…",
    completed: "Transfer complete · printer reports ready",
    cancelled: "Cancelled",
  };

  function setState(id: string, state: JobState, extra: Partial<JobSummary> = {}) {
    patchJob(id, { state, state_text: STEP_TEXT[state] ?? state, ...extra });
  }

  function pump() {
    if (!options.autoAdvance || held) return;
    if (jobs.some((j) => j.state === "connecting" || j.state === "sending" || j.state === "waiting_for_ready")) return;
    const next = sortedJobs().reverse().find((j) => j.state === "queued");
    if (!next) return;
    const id = next.id;
    setState(id, "connecting");
    later(options.stepMs, () => {
      if (jobs.find((j) => j.id === id)?.state !== "connecting") return;
      setState(id, "sending");
      const total = jobs.find((j) => j.id === id)?.bytes_total ?? 1;
      const steps = 5;
      for (let i = 1; i <= steps; i++) {
        later((options.stepMs * i) / steps, () => {
          if (jobs.find((j) => j.id === id)?.state !== "sending") return;
          const sent = Math.round((total * i) / steps);
          emit({ type: "job_progress", seq: ++seq, job_id: id, attempt_id: `${id}-a1`, bytes_sent: sent, bytes_total: total });
          patchJob(id, { bytes_sent: sent });
        });
      }
      later(options.stepMs + 50, () => {
        if (jobs.find((j) => j.id === id)?.state !== "sending") return;
        setState(id, "waiting_for_ready");
        later(options.stepMs, () => {
          if (jobs.find((j) => j.id === id)?.state !== "waiting_for_ready") return;
          setState(id, "completed", { content_retained: true });
          log("info", `Job ${id} completed`);
          pump();
        });
      });
    });
  }

  function makeJob(req: EnqueueRequest, retryOf?: string): JobSummary {
    const printer = printers.find((p) => p.id === req.printer_id);
    if (!printer) throw err("not_found", "That printer is no longer saved.");
    const profile = profileById(printer.profile_id)!;
    const snap = snapshots.get(req.snapshot_id);
    if (!snap) throw err("stale_preview", "The preview changed. Wait for the new preview, then print again.");
    const now = tick();
    const state: JobState = held ? "held" : "queued";
    return {
      id: newId("job"),
      title: req.title,
      state,
      state_text: STEP_TEXT[state] ?? state,
      printer_id: printer.id,
      printer_name: printer.name,
      profile_id: profile.id,
      profile_revision: profile.revision,
      created_ms: now,
      updated_ms: now,
      copies: req.copies,
      pages: 1,
      width: snap.width,
      height: snap.height,
      density: req.density,
      feed_mm: req.feed_mm,
      paper_mm: snap.length_mm * req.copies + req.feed_mm,
      bytes_total: Math.round((snap.width / 8) * snap.height * req.copies),
      bytes_sent: 0,
      partial_output_possible: false,
      content_retained: true,
      retry_of: retryOf,
      revision: 1,
    };
  }

  const snapshots = new Map<string, PreviewResult>();
  /** The preview each job was printed from, standing in for its saved page. */
  const jobPreviews = new Map<string, string>();

  function terminalCleanup() {
    jobs = jobs.filter((j) => !["completed", "transfer_unconfirmed", "accepted", "failed", "uncertain", "cancelled"].includes(j.state));
  }

  function seedPrinter() {
    const p = MOCK_PROFILES[0]!;
    printers = [
      {
        id: "printer-seed",
        name: "SC03h-TEST",
        backend_id: "mock-device-seed",
        address: "AA:BB:CC:00:11:22",
        profile_id: p.id,
        profile_revision: p.revision,
        profile_name: p.display_name,
        support: p.support,
        firmware: "1.0.3",
        last_seen_ms: Date.now(),
        settings: { density: p.default_density, feed_mm: p.feed.default_mm },
        profile_available: true,
        customized: false,
      },
    ];
    selected = "printer-seed";
  }
  if (options.seedPrinters) seedPrinter();

  async function lat<T>(value: () => T, ms = options.latencyMs): Promise<T> {
    await sleep(ms);
    return value();
  }

  function renderBody(req: PreviewRequest, profile: ProfileSummary) {
    const w = profile.width_dots;
    const s = req.source;
    let height = 120;
    let body = "";
    let warnings: string[] = [];
    if (s.kind === "text") {
      const margin = s.margin_dots;
      const charW = s.size_px * (s.family === "mono" ? 0.6 : 0.5);
      const perLine = Math.max(4, Math.floor((w - margin * 2) / charW));
      const lines: string[] = [];
      for (const raw of s.text.split("\n")) {
        const t = s.headings && raw.startsWith("# ") ? raw.slice(2) : raw;
        if (t.length === 0) lines.push("");
        for (let i = 0; i < t.length; i += perLine) lines.push(t.slice(i, i + perLine));
      }
      const lh = s.size_px * s.line_spacing;
      height = Math.max(32, Math.ceil(lines.length * lh + margin * 2));
      const anchor = s.align === "left" ? "start" : s.align === "center" ? "middle" : "end";
      const x = s.align === "left" ? margin : s.align === "center" ? w / 2 : w - margin;
      body = lines
        .map(
          (l, i) =>
            `<text x="${x}" y="${Math.round(margin + lh * i + s.size_px)}" font-size="${s.size_px}" text-anchor="${anchor}" font-family="${s.family === "mono" ? "monospace" : "sans-serif"}" font-weight="${s.bold ? 700 : 400}" fill="#000">${esc(l)}</text>`,
        )
        .join("");
      const missing = [...s.text].filter((c) => c.codePointAt(0)! > 0x2600 && c.codePointAt(0)! < 0x27bf);
      if (missing.length > 0) warnings = [`Some characters have no glyph in the selected font and will print as blanks: ${[...new Set(missing)].join(" ")}`];
    } else if (s.kind === "test_pattern") {
      height = 160;
      body =
        `<rect x="0" y="0" width="${w}" height="4" fill="#000"/>` +
        Array.from({ length: Math.floor(w / 16) }, (_, i) => `<rect x="${i * 16}" y="20" width="${i % 2 ? 4 : 8}" height="80" fill="#000"/>`).join("") +
        `<text x="8" y="140" font-size="20" fill="#000">${w} dots wide</text>`;
    } else {
      const rotated = s.adjust.rotation === "90" || s.adjust.rotation === "270";
      height = Math.round(w * (s.kind === "pdf" ? 1.4 : rotated ? 1.3 : 0.75));
      body =
        `<rect x="2" y="2" width="${w - 4}" height="${height - 4}" fill="none" stroke="#000" stroke-width="4"/>` +
        Array.from({ length: 12 }, (_, i) => `<line x1="0" y1="${(i * height) / 12}" x2="${w}" y2="${((i + 1) * height) / 12}" stroke="#000" stroke-width="${1 + (i % 3)}"/>`).join("") +
        `<text x="12" y="34" font-size="22" fill="#000">${esc(s.kind === "pdf" ? `PDF page ${s.page}` : "Image")} · ${s.adjust.tone}</text>`;
    }
    return { height, body, warnings };
  }

  const backend: MockBackend = {
    kind: "mock",
    calls,
    options,
    previewErrors: [],

    emit,
    seedJob(partial) {
      const printer = printers[0];
      const now = tick();
      const job: JobSummary = {
        id: newId("job"),
        title: "Seeded job",
        state_text: STEP_TEXT[partial.state] ?? partial.state,
        printer_id: printer?.id ?? "printer-seed",
        printer_name: printer?.name ?? "SC03h-TEST",
        profile_id: "sc03h",
        profile_revision: 1,
        created_ms: now,
        updated_ms: now,
        copies: 1,
        pages: 1,
        width: 384,
        height: 400,
        density: "normal",
        feed_mm: 12,
        paper_mm: 62,
        bytes_total: 19200,
        bytes_sent: 0,
        partial_output_possible: false,
        content_retained: true,
        revision: 1,
        ...partial,
      };
      putJob(job);
      return job;
    },

    getAppInfo: () => lat(() => infoApp),
    listProfiles: () => lat(() => [...MOCK_PROFILES, ...customProfiles].map((p) => ({ ...p }))),
    adapterStatus: () => lat(() => options.adapter),

    async startScan(request) {
      await sleep(options.latencyMs);
      const id = newId("scan");
      scanId = id;
      log("info", `Scan ${id} started`);
      if (options.adapter === "permission_not_asked" || options.adapter === "permission_pending") {
        // As if the macOS prompt went unanswered: the scan waits for it, then reports no access.
        later(options.scanMs, () => {
          if (scanId !== id) return;
          scanId = null;
          const error = err("permission_denied", "NuPrint does not have Bluetooth access yet.");
          emit({ type: "scan_finished", scan_id: id, found: 0, error });
        });
        return { scan_id: id, adapter: options.adapter };
      }
      if (options.adapter !== "ready") return { scan_id: id, adapter: options.adapter };
      const list: { at: number; dev: DiscoveredPrinter }[] = [];
      const [sc, demo] = MOCK_PROFILES as [ProfileSummary, ProfileSummary];
      const savedSc = printers.find((p) => p.backend_id === "mock-device-sc03h");
      list.push({
        at: 300,
        dev: {
          discovery_id: "mock-device-sc03h",
          name: "SC03h-TEST",
          address: "AA:BB:CC:00:11:22",
          rssi: -58,
          advertised_services: ["0000af30-0000-1000-8000-00805f9b34fb"],
          candidates: [match(sc)],
          saved_printer_id: savedSc?.id,
        },
      });
      list.push({
        at: 700,
        dev: {
          discovery_id: "mock-device-demo",
          name: "DemoWide-80",
          rssi: -72,
          advertised_services: [],
          candidates: [match(demo)],
        },
      });
      if (request.include_unknown) {
        list.push({
          at: 1000,
          dev: { discovery_id: "mock-device-unknown", rssi: -88, advertised_services: [], candidates: [] },
        });
      }
      for (const { at, dev } of list) {
        later(Math.min(at, options.scanMs), () => {
          if (scanId !== id) return;
          discovered.set(dev.discovery_id, dev);
          emit({ type: "scan_device", scan_id: id, device: dev });
        });
      }
      later(options.scanMs, () => {
        if (scanId !== id) return;
        scanId = null;
        emit({ type: "scan_finished", scan_id: id, found: list.length });
      });
      return { scan_id: id, adapter: options.adapter };
    },
    async stopScan(id) {
      await sleep(options.latencyMs);
      if (scanId === id) {
        scanId = null;
        emit({ type: "scan_finished", scan_id: id, found: discovered.size });
      }
    },
    async inspectDevice(discoveryId) {
      calls.inspect.push(discoveryId);
      await sleep(options.latencyMs + 100);
      const dev = discovered.get(discoveryId);
      if (!dev) throw err("device_not_found", "That device is no longer in range. Find printers again.");
      const inspection: DeviceInspection = {
        discovery_id: discoveryId,
        name: dev.name,
        services: [
          {
            uuid: "0000af30-0000-1000-8000-00805f9b34fb",
            characteristics: [
              { uuid: "0000af31-0000-1000-8000-00805f9b34fb", properties: ["write-without-response"] },
              { uuid: "0000af32-0000-1000-8000-00805f9b34fb", properties: ["notify"] },
            ],
          },
        ],
        compatible: dev.candidates,
        rejected: dev.candidates.length === 0 ? ["No profile matches this device's name and services."] : [],
        max_write_len: 20,
      };
      return inspection;
    },
    async cancelInspect() {
      calls.cancelInspect += 1;
    },
    async openBluetoothSettings() {
      calls.openBluetoothSettings += 1;
    },
    async setAppIcon(icon) {
      calls.setAppIcon.push(icon);
    },
    async setWindowTheme(mode) {
      calls.setWindowTheme.push(mode);
    },

    listPrinters: () => lat(() => printers.map((p) => ({ ...p }))),
    async savePrinter(request) {
      await sleep(options.latencyMs);
      calls.savePrinter.push({ ...request });
      const profile = profileById(request.profile_id);
      const dev = discovered.get(request.discovery_id);
      if (!profile || !dev) throw err("device_not_found", "That device is no longer in range. Find printers again.");
      if (profile.support === "candidate") throw err("unsupported_profile", "This model is not supported yet.");
      if (profile.support === "experimental" && !request.acknowledge_experimental) {
        throw err("experimental_not_acknowledged", "Confirm that you understand this printer is experimental before saving it.");
      }
      const printer: SavedPrinter = {
        id: newId("printer"),
        name: dev.name ?? "Unnamed printer",
        backend_id: dev.discovery_id,
        address: dev.address,
        profile_id: profile.id,
        profile_revision: profile.revision,
        profile_name: profile.display_name,
        support: profile.support,
        firmware: "1.0.3",
        last_seen_ms: Date.now(),
        settings: { density: profile.default_density, feed_mm: profile.feed.default_mm },
        profile_available: true,
        customized: false,
      };
      printers.push(printer);
      if (!selected) selected = printer.id;
      dev.saved_printer_id = printer.id;
      emit({ type: "printers_changed", seq: ++seq });
      log("info", `Saved printer ${printer.name}`);
      return printer;
    },
    async getPrinterProfile(printerId) {
      await sleep(options.latencyMs);
      const printer = printers.find((p) => p.id === printerId);
      if (!printer) throw err("not_found", "That printer is no longer saved.");
      const builtIn = MOCK_PROFILES.find((p) => p.id === (printerBases.get(printerId) ?? printer.profile_id))!;
      const docs = printerDocs.get(printerId) ?? { base: baseDoc(builtIn), current: baseDoc(builtIn) };
      printerDocs.set(printerId, docs);
      return {
        printer_id: printerId,
        base_profile_id: builtIn.id,
        base_display_name: builtIn.display_name,
        customized: printer.customized,
        base_json: docs.base,
        current_json: docs.current,
      };
    },
    async savePrinterProfile(request) {
      await sleep(options.latencyMs);
      calls.savePrinterProfile.push({ ...request });
      const i = printers.findIndex((p) => p.id === request.printer_id);
      if (i < 0) throw err("not_found", "That printer is no longer saved.");
      let edited: Record<string, unknown>;
      try {
        edited = JSON.parse(request.json) as Record<string, unknown>;
      } catch {
        throw err("invalid_input", "The settings are not valid JSON.");
      }
      const locked = Object.keys(edited).find((k) => LOCKED_KEYS.includes(k));
      if (locked) throw err("invalid_input", `"${locked}" cannot be changed.`);
      const chunk = (edited.transport as { max_chunk_bytes?: number } | undefined)?.max_chunk_bytes;
      if (typeof chunk === "number" && chunk > 512) throw err("invalid_input", "max_chunk_bytes must be at most 512.");
      const printer = printers[i]!;
      const builtIn = MOCK_PROFILES.find((p) => p.id === (printerBases.get(printer.id) ?? printer.profile_id))!;
      const docs = printerDocs.get(printer.id) ?? { base: baseDoc(builtIn), current: baseDoc(builtIn) };
      const current = JSON.stringify(edited, null, 2);
      printerDocs.set(printer.id, { base: docs.base, current });
      if (JSON.stringify(JSON.parse(docs.base)) === JSON.stringify(edited)) return this.resetPrinterProfile(printer.id);
      const id = `custom:${printer.id}`;
      const revision = (profileById(id)?.revision ?? 0) + 1;
      const summary: ProfileSummary = {
        ...builtIn,
        id,
        revision,
        support: "experimental",
        support_note: "Settings edited on this computer. They have not been verified on a real printer.",
      };
      const at = customProfiles.findIndex((p) => p.id === id);
      if (at >= 0) customProfiles[at] = summary;
      else customProfiles.push(summary);
      printerBases.set(printer.id, builtIn.id);
      printers[i] = {
        ...printer,
        profile_id: id,
        profile_revision: revision,
        support: "experimental",
        customized: true,
      };
      emit({ type: "printers_changed", seq: ++seq });
      return printers[i]!;
    },
    async resetPrinterProfile(printerId) {
      await sleep(options.latencyMs);
      calls.resetPrinterProfile.push(printerId);
      const i = printers.findIndex((p) => p.id === printerId);
      if (i < 0) throw err("not_found", "That printer is no longer saved.");
      const builtIn = MOCK_PROFILES.find((p) => p.id === (printerBases.get(printerId) ?? printers[i]!.profile_id))!;
      printerBases.delete(printerId);
      printerDocs.delete(printerId);
      printers[i] = {
        ...printers[i]!,
        profile_id: builtIn.id,
        profile_revision: builtIn.revision,
        support: builtIn.support,
        customized: false,
      };
      emit({ type: "printers_changed", seq: ++seq });
      return printers[i]!;
    },
    async exportPrinterProfile(printerId) {
      await sleep(options.latencyMs);
      calls.exportPrinterProfile.push(printerId);
      return options.savePath !== null;
    },
    async removePrinter(printerId) {
      await sleep(options.latencyMs);
      printers = printers.filter((p) => p.id !== printerId);
      if (selected === printerId) selected = printers[0]?.id ?? null;
      emit({ type: "printers_changed", seq: ++seq });
    },
    async updatePrinterSettings(printerId, settings) {
      await sleep(options.latencyMs);
      calls.updateSettings.push({ printerId, density: settings.density, feed_mm: settings.feed_mm });
      const i = printers.findIndex((p) => p.id === printerId);
      if (i < 0) throw err("not_found", "That printer is no longer saved.");
      printers[i] = { ...printers[i]!, settings: { ...settings } };
      emit({ type: "printers_changed", seq: ++seq });
      return printers[i]!;
    },
    getSelectedPrinter: () => lat(() => selected),
    async selectPrinter(printerId) {
      await sleep(options.latencyMs);
      selected = printerId;
      emit({ type: "printers_changed", seq: ++seq });
    },
    async checkPrinter(printerId) {
      await sleep(options.latencyMs + 150);
      const p = printers.find((x) => x.id === printerId);
      if (!p) throw err("not_found", "That printer is no longer saved.");
      lastCheck = {
        printer_id: printerId,
        ready: true,
        status_text: "Ready · paper present · cover closed",
        flags: [],
        raw_status_hex: "00",
        firmware: p.firmware,
        max_write_len: 20,
        services: [
          {
            uuid: "0000af30-0000-1000-8000-00805f9b34fb",
            characteristics: [{ uuid: "0000af31-0000-1000-8000-00805f9b34fb", properties: ["write-without-response"] }],
          },
        ],
        notifications: ["0000af32-0000-1000-8000-00805f9b34fb"],
        checked_ms: Date.now(),
      };
      log("info", `Checked ${p.name}: ready`);
      return lastCheck;
    },

    async openSource() {
      await sleep(options.latencyMs);
      const path = options.openPath;
      if (!path) return null;
      const name = path.split(/[\\/]/).pop() ?? path;
      const isPdf = /\.pdf$/i.test(name);
      const info: SourceInfo = {
        source_id: newId("src"),
        kind: isPdf ? "pdf" : "image",
        name,
        bytes: 184_320,
        page_count: isPdf ? 3 : undefined,
      };
      sources.set(info.source_id, { info, locked: /locked/i.test(name) });
      return info;
    },
    async renderPreview(request) {
      const callIndex = ++previewCalls;
      calls.renderPreview.push(request);
      maxGeneration = Math.max(maxGeneration, request.generation);
      const ms = options.previewLatencyMs ? options.previewLatencyMs(callIndex) : options.latencyMs;
      await sleep(ms);
      const forced = backend.previewErrors.shift();
      if (forced) throw forced;
      if (request.generation < maxGeneration) throw err("stale_preview", "A newer preview was requested.");
      const profile = profileById(request.profile_id);
      if (!profile) throw err("unsupported_profile", "That printer profile is not available.");
      const s = request.source;
      if (s.kind === "pdf") {
        const src = sources.get(s.source_id);
        if (src?.locked && s.password !== "secret") {
          throw err(
            "pdf_password_required",
            s.password ? "That password is not correct." : "This PDF is password protected. Enter its password to preview it.",
          );
        }
      }
      const { height, body, warnings } = renderBody(request, profile);
      const result: PreviewResult = {
        generation: request.generation,
        snapshot_id: newId("snap"),
        profile_id: profile.id,
        profile_revision: profile.revision,
        width: profile.width_dots,
        height,
        length_mm: height / profile.dots_per_mm,
        sha256: "0".repeat(64),
        black_ratio: 0.12,
        warnings,
        preview_path: svgDataUrl(profile.width_dots, height, body),
      };
      snapshots.set(result.snapshot_id, result);
      return result;
    },
    async savePreviewPng(snapshotId) {
      await sleep(options.latencyMs);
      const path = options.savePath;
      if (!path) return false;
      if (!snapshots.has(snapshotId)) throw err("stale_preview", "That preview is no longer available.");
      calls.savePng.push({ snapshotId, path });
      return true;
    },
    previewUrl: (p) => p,
    jobPreviewUrl: (id) => {
      const job = jobs.find((j) => j.id === id);
      return (job && jobPreviews.get(id)) || "";
    },

    async enqueueJob(request) {
      await sleep(options.latencyMs);
      calls.enqueue.push({ ...request });
      const existing = idempotent.get(request.idempotency_key);
      if (existing) return existing;
      const job = makeJob(request);
      jobPreviews.set(job.id, snapshots.get(request.snapshot_id)?.preview_path ?? "");
      idempotent.set(request.idempotency_key, job);
      putJob(job);
      log("info", `Queued job ${job.id}: ${job.title}`);
      pump();
      return job;
    },
    listJobs: () => lat(snapshot),
    getJob: (id) =>
      lat(() => {
        const j = jobs.find((x) => x.id === id);
        if (!j) throw err("not_found", "That job no longer exists.");
        return j;
      }),
    async cancelJob(id) {
      await sleep(options.latencyMs);
      calls.cancel.push(id);
      const j = jobs.find((x) => x.id === id);
      if (!j) throw err("not_found", "That job no longer exists.");
      const partial = j.state === "sending" || j.state === "waiting_for_ready";
      setState(id, "cancelled", { partial_output_possible: partial, state_text: partial ? "Cancelled. Data already in the printer may still print." : "Cancelled" });
      pump();
    },
    async retryJob(id, key) {
      await sleep(options.latencyMs);
      const existing = idempotent.get(key);
      if (existing) return existing;
      const j = jobs.find((x) => x.id === id);
      if (!j || !j.content_retained) throw err("not_found", "The content for this job was not kept, so it cannot be printed again.");
      const now = tick();
      const state: JobState = held ? "held" : "queued";
      const job: JobSummary = {
        ...j,
        id: newId("job"),
        state,
        state_text: STEP_TEXT[state] ?? state,
        created_ms: now,
        updated_ms: now,
        bytes_sent: 0,
        error: undefined,
        partial_output_possible: false,
        retry_of: j.id,
        revision: 1,
      };
      jobPreviews.set(job.id, jobPreviews.get(j.id) ?? "");
      idempotent.set(key, job);
      putJob(job);
      pump();
      return job;
    },
    async removeJob(id) {
      await sleep(options.latencyMs);
      jobs = jobs.filter((j) => j.id !== id);
      queueRevision++;
      emit({ type: "job_removed", seq: ++seq, job_id: id });
    },
    async clearHistory() {
      await sleep(options.latencyMs);
      const removed = jobs.filter((j) => ["completed", "transfer_unconfirmed", "accepted", "failed", "uncertain", "cancelled"].includes(j.state));
      terminalCleanup();
      queueRevision++;
      for (const j of removed) emit({ type: "job_removed", seq: ++seq, job_id: j.id });
    },
    async holdQueue() {
      await sleep(options.latencyMs);
      held = true;
      queueRevision++;
      return snapshot();
    },
    async resumeQueue() {
      await sleep(options.latencyMs);
      held = false;
      queueRevision++;
      for (const j of jobs.filter((x) => x.state === "held")) setState(j.id, "queued");
      pump();
      return snapshot();
    },

    getLogs: () => lat(() => [...logs]),
    async diagnosticsReport(includeIdentifiers) {
      await sleep(options.latencyMs);
      const report: DiagnosticsReport = {
        generated_ms: Date.now(),
        app: infoApp,
        adapter: options.adapter,
        profiles: MOCK_PROFILES,
        printers: includeIdentifiers ? printers : printers.map((p) => ({ ...p, name: "(hidden)", address: undefined, backend_id: "(hidden)" })),
        last_check: lastCheck,
        recent_jobs: sortedJobs().slice(0, 20),
        log: [...logs],
        includes_identifiers: includeIdentifiers,
      };
      return report;
    },
    async exportDiagnostics(includeIdentifiers) {
      await sleep(options.latencyMs);
      if (!options.savePath) return false;
      calls.exportDiagnostics.push({ includeIdentifiers });
      return true;
    },

    async checkForUpdate() {
      await sleep(options.latencyMs);
      calls.checkForUpdate += 1;
      return options.update;
    },
    async installUpdate() {
      await sleep(options.latencyMs);
      if (!options.update) throw err("not_found", "Check for updates first.");
      if (jobs.some((j) => isActive(j.state))) {
        throw err("printer_busy", "A job is printing. Install the update when it has finished.");
      }
      calls.installUpdate += 1;
    },
    async openReleasePage() {
      calls.openReleasePage += 1;
    },

    async subscribe(handler) {
      handlers.add(handler);
      return () => {
        handlers.delete(handler);
      };
    },

  };

  return backend;
}
