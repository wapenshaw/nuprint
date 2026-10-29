import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api } from "../api";
import type { AppEvent } from "../bindings/AppEvent";
import type { PrinterCheck } from "../bindings/PrinterCheck";
import type { ProfileSummary } from "../bindings/ProfileSummary";
import type { SavedPrinter } from "../bindings/SavedPrinter";
import { errorMessage, toAppError } from "../lib/errors";
import { isOpenJob } from "../lib/labels";
import { loadAppIcon, loadModePreference, saveAppIcon, type AppIconId } from "../lib/theme";
import { initialData, reducer, type AppData, type StatusMessage, type StatusTone } from "./reducer";

/** The settings window's sections. */
export type SettingsSection = "printers" | "appearance" | "diagnostics";

/**
 * What is open over the workspace, which is always there: nothing, the queue panel, or the
 * settings window at one of its sections.
 */
export type View = "workspace" | "queue" | SettingsSection;

export interface AppActions {
  notify: (text: string, tone?: StatusTone, action?: StatusMessage["action"], jobId?: string) => void;
  dismissStatus: () => void;
  reportError: (e: unknown, prefix?: string) => void;
  refreshPrinters: () => Promise<void>;
  refreshQueue: () => Promise<void>;
  selectPrinter: (id: string | null) => Promise<void>;
  checkAdapter: () => Promise<void>;
  startScan: (includeUnknown: boolean) => Promise<void>;
  stopScan: () => Promise<void>;
  checkPrinter: (id: string) => Promise<PrinterCheck>;
  /** Check a printer's connection, tracking it so every view shows the same light. */
  runCheck: (id: string) => Promise<void>;
  setPrinter: (printer: SavedPrinter) => void;
}

export interface AppContextValue extends AppActions {
  data: AppData;
  view: View;
  setView: (v: View) => void;
  selectedPrinter: SavedPrinter | null;
  /** Profile of the selected printer, if it is known to this build. */
  selectedProfile: ProfileSummary | null;
  /** Profile used for previews: the selected printer's, else the first available. */
  previewProfile: ProfileSummary | null;
  openJobCount: number;
  /** The saved printer whose connection is being checked right now, if any. */
  checking: string | null;
  /** Why the last check of a printer failed, by printer id. */
  checkErrors: Record<string, string>;
  appIcon: AppIconId;
  chooseAppIcon: (icon: AppIconId) => void;
}

const Ctx = createContext<AppContextValue | null>(null);

export function useApp(): AppContextValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("useApp must be used inside <AppProvider>");
  return v;
}

let statusId = 0;

export function AppProvider({ children }: { children: ReactNode }) {
  const [data, dispatch] = useReducer(reducer, initialData);
  const [view, setView] = useState<View>("workspace");
  const [checking, setChecking] = useState<string | null>(null);
  const [checkErrors, setCheckErrors] = useState<Record<string, string>>({});
  const [appIcon, setAppIcon] = useState<AppIconId>(loadAppIcon);
  const dataRef = useRef(data);
  dataRef.current = data;

  const notify = useCallback<AppActions["notify"]>((text, tone = "info", action, jobId) => {
    dispatch({ type: "status", message: { id: ++statusId, tone, text, action, jobId } });
  }, []);
  const dismissStatus = useCallback(() => dispatch({ type: "status", message: null }), []);
  const reportError = useCallback<AppActions["reportError"]>(
    (e, prefix) => {
      const msg = errorMessage(e);
      notify(prefix ? `${prefix} ${msg}` : msg, "error");
    },
    [notify],
  );

  const refreshQueue = useCallback(async () => {
    const snap = await api().listJobs();
    dispatch({ type: "queue", jobs: snap.jobs, held: snap.held });
  }, []);

  const refreshPrinters = useCallback(async () => {
    // Profiles are re-read with the printers: editing a printer's settings adds a profile.
    const [printers, selected, profiles] = await Promise.all([
      api().listPrinters(),
      api().getSelectedPrinter(),
      api().listProfiles(),
    ]);
    let selectedPrinterId = selected;
    // Nothing (valid) selected but printers exist: default to the first one.
    if ((!selected || !printers.some((p) => p.id === selected)) && printers.length > 0) {
      selectedPrinterId = printers[0]!.id;
      try {
        await api().selectPrinter(selectedPrinterId);
      } catch {
        /* the next refresh will try again */
      }
    } else if (selected && !printers.some((p) => p.id === selected)) {
      selectedPrinterId = null;
    }
    dispatch({ type: "printers", printers, profiles, selectedPrinterId });
  }, []);

  const selectPrinter = useCallback(
    async (id: string | null) => {
      dispatch({ type: "selected", id });
      try {
        await api().selectPrinter(id);
      } catch (e) {
        reportError(e, "Could not switch printers.");
        await refreshPrinters().catch(() => undefined);
      }
    },
    [refreshPrinters, reportError],
  );

  const checkAdapter = useCallback(async () => {
    try {
      dispatch({ type: "adapter", adapter: await api().adapterStatus() });
    } catch (e) {
      reportError(e, "Could not read Bluetooth status.");
    }
  }, [reportError]);

  const startScan = useCallback(
    async (includeUnknown: boolean) => {
      dispatch({ type: "scan_starting" });
      try {
        const started = await api().startScan({ duration_s: 8, include_unknown: includeUnknown });
        dispatch({ type: "scan_started", scanId: started.scan_id, adapter: started.adapter });
        // While macOS asks for permission the scan keeps running and waits for the answer.
        const asking = started.adapter === "permission_not_asked" || started.adapter === "permission_pending";
        if (started.adapter !== "ready" && !asking) dispatch({ type: "scan_stopped" });
      } catch (e) {
        dispatch({ type: "scan_failed", error: toAppError(e) });
      }
    },
    [],
  );

  const stopScan = useCallback(async () => {
    const id = dataRef.current.scan.scanId;
    dispatch({ type: "scan_stopped" });
    if (!id) return;
    try {
      await api().stopScan(id);
    } catch (e) {
      reportError(e, "Could not stop the scan.");
    }
  }, [reportError]);

  const checkPrinter = useCallback(async (id: string) => {
    const check = await api().checkPrinter(id);
    dispatch({ type: "check", check });
    return check;
  }, []);

  const runCheck = useCallback(
    async (id: string) => {
      setChecking(id);
      setCheckErrors(({ [id]: _drop, ...rest }) => rest);
      try {
        await checkPrinter(id);
      } catch (e) {
        setCheckErrors((prev) => ({ ...prev, [id]: toAppError(e).message }));
      } finally {
        setChecking(null);
      }
    },
    [checkPrinter],
  );

  // The host shows the chosen icon in the Dock or taskbar. Purely cosmetic: a failure leaves
  // the bundle's icon in place.
  useEffect(() => {
    api()
      .setAppIcon(appIcon)
      .catch(() => undefined);
  }, [appIcon]);
  // A Light or Dark pick from an earlier run applies to the title bar too.
  useEffect(() => {
    const mode = loadModePreference();
    if (mode !== "system") api().setWindowTheme(mode).catch(() => undefined);
  }, []);
  const chooseAppIcon = useCallback((icon: AppIconId) => {
    saveAppIcon(icon);
    setAppIcon(icon);
  }, []);

  const setPrinter = useCallback((printer: SavedPrinter) => dispatch({ type: "printer_updated", printer }), []);

  // Initial load. Subscribe first but hold events back until the durable snapshot is applied;
  // buffered queue events already covered by the snapshot's `seq` are dropped.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let loaded = false;
    const buffer: AppEvent[] = [];

    const handle = (event: AppEvent) => {
      if (event.type === "printers_changed") {
        refreshPrinters().catch(() => undefined);
        return;
      }
      dispatch({ type: "event", event });
    };

    api()
      .subscribe((event) => {
        if (cancelled) return;
        if (!loaded) buffer.push(event);
        else handle(event);
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);

    (async () => {
      try {
        const b = api();
        const [info, profiles, queue, printers, selected, logs, adapter] = await Promise.all([
          b.getAppInfo(),
          b.listProfiles(),
          b.listJobs(),
          b.listPrinters(),
          b.getSelectedPrinter(),
          b.getLogs(),
          b.adapterStatus().catch(() => "unknown" as const),
        ]);
        if (cancelled) return;
        const effectiveSelected =
          selected && printers.some((p) => p.id === selected) ? selected : (printers[0]?.id ?? null);
        if (effectiveSelected && effectiveSelected !== selected) b.selectPrinter(effectiveSelected).catch(() => undefined);
        dispatch({
          type: "loaded",
          info,
          profiles,
          printers,
          selectedPrinterId: effectiveSelected,
          jobs: queue.jobs,
          queueHeld: queue.held,
          logs,
          adapter,
        });
        loaded = true;
        // Queue events up to the snapshot's sequence number are already reflected in it.
        for (const e of buffer.splice(0)) {
          const queueEvent = e.type === "job_changed" || e.type === "job_removed" || e.type === "job_progress";
          if (!queueEvent || e.seq > queue.seq) handle(e);
        }
      } catch (e) {
        if (!cancelled) dispatch({ type: "load_failed", error: toAppError(e) });
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refreshPrinters]);

  const value = useMemo<AppContextValue>(() => {
    const selectedPrinter = data.printers.find((p) => p.id === data.selectedPrinterId) ?? null;
    const selectedProfile = selectedPrinter
      ? (data.profiles.find((p) => p.id === selectedPrinter.profile_id) ?? null)
      : null;
    const previewProfile = selectedProfile ?? (selectedPrinter ? null : (data.profiles[0] ?? null));
    return {
      data,
      view,
      setView,
      selectedPrinter,
      selectedProfile,
      previewProfile,
      openJobCount: data.jobs.filter((j) => isOpenJob(j.state)).length,
      checking,
      checkErrors,
      appIcon,
      chooseAppIcon,
      runCheck,
      notify,
      dismissStatus,
      reportError,
      refreshPrinters,
      refreshQueue,
      selectPrinter,
      checkAdapter,
      startScan,
      stopScan,
      checkPrinter,
      setPrinter,
    };
  }, [
    data,
    view,
    notify,
    dismissStatus,
    reportError,
    refreshPrinters,
    refreshQueue,
    selectPrinter,
    checkAdapter,
    startScan,
    stopScan,
    checkPrinter,
    setPrinter,
    checking,
    checkErrors,
    appIcon,
    chooseAppIcon,
    runCheck,
  ]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
