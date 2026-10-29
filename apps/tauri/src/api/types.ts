import type { AdapterStatus } from "../bindings/AdapterStatus";
import type { AppEvent } from "../bindings/AppEvent";
import type { AppInfo } from "../bindings/AppInfo";
import type { DeviceInspection } from "../bindings/DeviceInspection";
import type { DiagnosticsReport } from "../bindings/DiagnosticsReport";
import type { EnqueueRequest } from "../bindings/EnqueueRequest";
import type { JobSummary } from "../bindings/JobSummary";
import type { LogLine } from "../bindings/LogLine";
import type { PrinterProfileDoc } from "../bindings/PrinterProfileDoc";
import type { PreviewRequest } from "../bindings/PreviewRequest";
import type { PreviewResult } from "../bindings/PreviewResult";
import type { PrinterCheck } from "../bindings/PrinterCheck";
import type { PrinterSettings } from "../bindings/PrinterSettings";
import type { ProfileSummary } from "../bindings/ProfileSummary";
import type { QueueSnapshot } from "../bindings/QueueSnapshot";
import type { SavePrinterProfileRequest } from "../bindings/SavePrinterProfileRequest";
import type { SavePrinterRequest } from "../bindings/SavePrinterRequest";
import type { SavedPrinter } from "../bindings/SavedPrinter";
import type { ScanStarted } from "../bindings/ScanStarted";
import type { SourceInfo } from "../bindings/SourceInfo";
import type { StartScanRequest } from "../bindings/StartScanRequest";
import type { UpdateInfo } from "../bindings/UpdateInfo";

export type Unsubscribe = () => void;

/**
 * Everything the UI needs from the host. The Tauri implementation maps each method onto
 * one `invoke` command; the mock implementation is used outside Tauri (browser dev, tests).
 * The frontend never builds printer packets or rasters: it only asks Rust to.
 */
export interface Backend {
  readonly kind: "tauri" | "mock";

  getAppInfo(): Promise<AppInfo>;
  listProfiles(): Promise<ProfileSummary[]>;
  adapterStatus(): Promise<AdapterStatus>;

  startScan(request: StartScanRequest): Promise<ScanStarted>;
  stopScan(scanId: string): Promise<void>;
  inspectDevice(discoveryId: string): Promise<DeviceInspection>;
  /** Stops any running inspection so Bluetooth is free for the next action. */
  cancelInspect(): Promise<void>;
  /** Opens the macOS settings where Bluetooth access is granted to NuPrint. */
  openBluetoothSettings(): Promise<void>;
  /** Shows the chosen app icon (an `AppIconId`) in the Dock, or on the window and taskbar. */
  setAppIcon(icon: string): Promise<void>;
  /** Makes the native title bar light or dark to match a mode picked in Settings; null follows the system. */
  setWindowTheme(mode: "light" | "dark" | null): Promise<void>;

  listPrinters(): Promise<SavedPrinter[]>;
  savePrinter(request: SavePrinterRequest): Promise<SavedPrinter>;
  removePrinter(printerId: string): Promise<void>;
  /** The settings a user may edit for this printer (Printers → Advanced), with the built-in ones. */
  getPrinterProfile(printerId: string): Promise<PrinterProfileDoc>;
  savePrinterProfile(request: SavePrinterProfileRequest): Promise<SavedPrinter>;
  /** Back to the built-in settings. */
  resetPrinterProfile(printerId: string): Promise<SavedPrinter>;
  /** Rust shows the native save dialog and writes the profile file; false when cancelled. */
  exportPrinterProfile(printerId: string): Promise<boolean>;
  updatePrinterSettings(printerId: string, settings: PrinterSettings): Promise<SavedPrinter>;
  getSelectedPrinter(): Promise<string | null>;
  selectPrinter(printerId: string | null): Promise<void>;
  checkPrinter(printerId: string): Promise<PrinterCheck>;

  /** Rust shows the native open dialog and imports the chosen file; null when cancelled. */
  openSource(): Promise<SourceInfo | null>;
  renderPreview(request: PreviewRequest): Promise<PreviewResult>;
  /** Rust shows the native save dialog; false when cancelled. */
  savePreviewPng(snapshotId: string): Promise<boolean>;
  /** Turns `PreviewResult.preview_path` into an `<img src>` URL. */
  previewUrl(previewPath: string): string;
  /** A saved job's first page, while its content is kept (completed jobs keep none). */
  jobPreviewUrl(jobId: string): string;

  enqueueJob(request: EnqueueRequest): Promise<JobSummary>;
  listJobs(): Promise<QueueSnapshot>;
  getJob(jobId: string): Promise<JobSummary>;
  cancelJob(jobId: string): Promise<void>;
  retryJob(jobId: string, idempotencyKey: string): Promise<JobSummary>;
  removeJob(jobId: string): Promise<void>;
  clearHistory(): Promise<void>;
  holdQueue(): Promise<QueueSnapshot>;
  resumeQueue(): Promise<QueueSnapshot>;

  getLogs(): Promise<LogLine[]>;
  diagnosticsReport(includeIdentifiers: boolean): Promise<DiagnosticsReport>;
  /** Rust shows the native save dialog; false when cancelled. */
  exportDiagnostics(includeIdentifiers: boolean): Promise<boolean>;

  /** Asks for the newest release; null when this is it. Only ever called because the user asked. */
  checkForUpdate(): Promise<UpdateInfo | null>;
  /** Downloads the update the last check found, installs it and restarts. Refused while a job prints. */
  installUpdate(): Promise<void>;
  /** Opens the release page of the update the last check found in the browser. */
  openReleasePage(): Promise<void>;

  /** Single event channel `nuprint://event`. */
  subscribe(handler: (event: AppEvent) => void): Promise<Unsubscribe>;
}
