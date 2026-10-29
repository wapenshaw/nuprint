import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppEvent } from "../bindings/AppEvent";
import type { Backend } from "./types";

export const EVENT_CHANNEL = "nuprint://event";

/**
 * Production backend: every method is one Tauri command. Args are camelCase keys.
 * Native file dialogs run in Rust, so file-system paths never cross the IPC boundary.
 */
export function createTauriBackend(): Backend {
  return {
    kind: "tauri",

    getAppInfo: () => invoke("get_app_info"),
    listProfiles: () => invoke("list_profiles"),
    adapterStatus: () => invoke("adapter_status"),

    startScan: (request) => invoke("start_scan", { request }),
    stopScan: (scanId) => invoke("stop_scan", { scanId }),
    inspectDevice: (discoveryId) => invoke("inspect_device", { discoveryId }),
    cancelInspect: () => invoke("cancel_inspect"),
    openBluetoothSettings: () => invoke("open_bluetooth_settings"),
    setAppIcon: (icon) => invoke("set_app_icon", { icon }),
    setWindowTheme: (mode) => invoke("set_window_theme", { mode }),

    listPrinters: () => invoke("list_printers"),
    savePrinter: (request) => invoke("save_printer", { request }),
    removePrinter: (printerId) => invoke("remove_printer", { printerId }),
    getPrinterProfile: (printerId) => invoke("get_printer_profile", { printerId }),
    savePrinterProfile: (request) => invoke("save_printer_profile", { request }),
    resetPrinterProfile: (printerId) => invoke("reset_printer_profile", { printerId }),
    exportPrinterProfile: (printerId) => invoke("export_printer_profile", { printerId }),
    updatePrinterSettings: (printerId, settings) => invoke("update_printer_settings", { printerId, settings }),
    getSelectedPrinter: () => invoke("get_selected_printer"),
    selectPrinter: (printerId) => invoke("select_printer", { printerId }),
    checkPrinter: (printerId) => invoke("check_printer", { printerId }),

    openSource: () => invoke("open_source"),
    renderPreview: (request) => invoke("render_preview", { request }),
    savePreviewPng: (snapshotId) => invoke("save_preview_png", { snapshotId }),
    previewUrl: (previewPath) => convertFileSrc(previewPath, "nupreview"),
    jobPreviewUrl: (jobId) => convertFileSrc(`job-${jobId}.png`, "nupreview"),

    enqueueJob: (request) => invoke("enqueue_job", { request }),
    listJobs: () => invoke("list_jobs"),
    getJob: (jobId) => invoke("get_job", { jobId }),
    cancelJob: (jobId) => invoke("cancel_job", { jobId }),
    retryJob: (jobId, idempotencyKey) => invoke("retry_job", { jobId, idempotencyKey }),
    removeJob: (jobId) => invoke("remove_job", { jobId }),
    clearHistory: () => invoke("clear_history"),
    holdQueue: () => invoke("hold_queue"),
    resumeQueue: () => invoke("resume_queue"),

    getLogs: () => invoke("get_logs"),
    diagnosticsReport: (includeIdentifiers) => invoke("diagnostics_report", { includeIdentifiers }),
    exportDiagnostics: (includeIdentifiers) => invoke("export_diagnostics", { includeIdentifiers }),

    checkForUpdate: () => invoke("check_for_update"),
    installUpdate: () => invoke("install_update"),
    openReleasePage: () => invoke("open_release_page"),

    subscribe: (handler) => listen<AppEvent>(EVENT_CHANNEL, (e) => handler(e.payload)),

  };
}
