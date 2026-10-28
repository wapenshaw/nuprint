import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import { api } from "../api";
import type { Density } from "../bindings/Density";
import type { PreviewResult } from "../bindings/PreviewResult";
import { clampInt, paperEstimateMm } from "../lib/estimate";
import { newIdempotencyKey } from "../lib/uuid";
import { useApp } from "./AppStore";
import { usePreview, type PreviewState } from "./usePreview";
import { buildSourceSpec, initialWorkspace, jobTitle, type WorkspaceState } from "./workspace";

export const FEED_PERSIST_DELAY_MS = 400;

export interface PrintSettings {
  density: Density;
  feedText: string;
  /** The feed that will actually be sent: integer within the profile bounds. */
  feedMm: number;
  copiesText: string;
  copies: number;
}

export interface WorkspaceContextValue {
  ws: WorkspaceState;
  setWs: Dispatch<SetStateAction<WorkspaceState>>;
  preview: PreviewState;
  settings: PrintSettings | null;
  setDensity: (d: Density) => void;
  setFeedText: (text: string) => void;
  normalizeFeed: () => void;
  setCopiesText: (text: string) => void;
  normalizeCopies: () => void;
  /** Total paper for the displayed preview, or null when there is no preview. */
  paperMm: number | null;
  /** Why Print is unavailable right now (null when it can be pressed). */
  printBlockedReason: string | null;
  printing: boolean;
  print: () => Promise<void>;
  openFile: () => Promise<void>;
  savePng: () => Promise<void>;
}

/** A second Print click this soon after an enqueue is the tail of a double-click, not a new job. */
const REPRINT_GUARD_MS = 800;

const Ctx = createContext<WorkspaceContextValue | null>(null);

export function useWorkspace(): WorkspaceContextValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("useWorkspace must be used inside <WorkspaceProvider>");
  return v;
}

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const app = useApp();
  const { selectedPrinter: printer, selectedProfile: profile, previewProfile, notify, reportError, setPrinter } = app;

  const [ws, setWs] = useState<WorkspaceState>(initialWorkspace);
  const spec = useMemo(() => buildSourceSpec(ws), [ws]);
  const preview = usePreview(previewProfile?.id ?? null, spec, previewProfile?.revision);

  // ---- print settings (derived from the selected printer, edited locally, persisted per printer)
  const syncKey = printer && profile ? `${printer.id}:${profile.id}:${profile.revision}` : null;
  const [synced, setSynced] = useState<{
    key: string | null;
    density: Density;
    feedText: string;
    copiesText: string;
  }>({ key: null, density: "normal", feedText: "", copiesText: "1" });

  if (synced.key !== syncKey) {
    if (printer && profile) {
      const density = profile.densities.includes(printer.settings.density) ? printer.settings.density : profile.default_density;
      setSynced({
        key: syncKey,
        density,
        feedText: String(clampInt(printer.settings.feed_mm, profile.feed.min_mm, profile.feed.max_mm, profile.feed.default_mm)),
        copiesText: "1",
      });
    } else {
      setSynced({ key: null, density: "normal", feedText: "", copiesText: "1" });
    }
  }

  const settings = useMemo<PrintSettings | null>(() => {
    if (!printer || !profile || synced.key !== syncKey) return null;
    return {
      density: synced.density,
      feedText: synced.feedText,
      feedMm: clampInt(synced.feedText, profile.feed.min_mm, profile.feed.max_mm, printer.settings.feed_mm),
      copiesText: synced.copiesText,
      copies: clampInt(synced.copiesText, 1, profile.max_copies, 1),
    };
  }, [printer, profile, synced, syncKey]);

  const persistTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (persistTimer.current) clearTimeout(persistTimer.current);
    },
    [],
  );

  const persist = useCallback(
    (printerId: string, density: Density, feedMm: number) => {
      api()
        .updatePrinterSettings(printerId, { density, feed_mm: feedMm })
        .then(setPrinter)
        .catch((e) => reportError(e, "Could not save the printer settings."));
    },
    [reportError, setPrinter],
  );

  const setDensity = useCallback(
    (density: Density) => {
      if (!printer || !settings) return;
      setSynced((s) => ({ ...s, density }));
      persist(printer.id, density, settings.feedMm);
    },
    [persist, printer, settings],
  );

  const setFeedText = useCallback(
    (text: string) => {
      if (!printer || !profile) return;
      if (text.trim() === "") {
        setSynced((s) => ({ ...s, feedText: "" }));
        return;
      }
      const clamped = clampInt(text, profile.feed.min_mm, profile.feed.max_mm, profile.feed.default_mm);
      setSynced((s) => ({ ...s, feedText: String(clamped) }));
      if (persistTimer.current) clearTimeout(persistTimer.current);
      const density = settings?.density ?? profile.default_density;
      persistTimer.current = setTimeout(() => persist(printer.id, density, clamped), FEED_PERSIST_DELAY_MS);
    },
    [persist, printer, profile, settings?.density],
  );

  const normalizeFeed = useCallback(() => {
    if (!printer || !profile) return;
    setSynced((s) => ({
      ...s,
      feedText: String(clampInt(s.feedText, profile.feed.min_mm, profile.feed.max_mm, printer.settings.feed_mm)),
    }));
  }, [printer, profile]);

  const setCopiesText = useCallback(
    (text: string) => {
      if (!profile) return;
      if (text.trim() === "") {
        setSynced((s) => ({ ...s, copiesText: "" }));
        return;
      }
      setSynced((s) => ({ ...s, copiesText: String(clampInt(text, 1, profile.max_copies, 1)) }));
    },
    [profile],
  );

  const normalizeCopies = useCallback(() => {
    if (!profile) return;
    setSynced((s) => ({ ...s, copiesText: String(clampInt(s.copiesText, 1, profile.max_copies, 1)) }));
  }, [profile]);

  // ---- printing
  const result: PreviewResult | null = preview.result;
  const paperMm = result && settings ? paperEstimateMm(result.length_mm, settings.copies, settings.feedMm) : null;

  let printBlockedReason: string | null = null;
  if (!printer) printBlockedReason = "No printer is set up yet. Use Find printers… in the printer menu.";
  else if (!profile) printBlockedReason = "This printer's profile is not available in this version of NuPrint.";
  else if (preview.idle) printBlockedReason = "Nothing to print yet.";
  else if (preview.rendering) printBlockedReason = "Updating the preview…";
  else if (preview.error) printBlockedReason = "Fix the preview first.";
  else if (!result || !preview.current) printBlockedReason = "Updating the preview…";
  else if (result.profile_id !== printer.profile_id) printBlockedReason = "Updating the preview…";

  const printingRef = useRef(false);
  const [printing, setPrinting] = useState(false);

  const print = useCallback(async () => {
    if (printingRef.current) return;
    if (!printer) {
      notify("No printer is set up yet. Use Find printers… in the printer menu.", "info", {
        label: "Printer settings",
        target: "printers",
      });
      return;
    }
    if (printBlockedReason || !result || !settings) {
      notify(printBlockedReason ?? "The preview is not ready yet.", "info");
      return;
    }
    printingRef.current = true;
    setPrinting(true);
    try {
      const job = await api().enqueueJob({
        snapshot_id: result.snapshot_id,
        printer_id: printer.id,
        copies: settings.copies,
        density: settings.density,
        feed_mm: settings.feedMm,
        idempotency_key: newIdempotencyKey(),
        title: jobTitle(ws),
      });
      notify(`Added "${job.title}" to the print queue.`, "success", { label: "View queue", target: "queue" }, job.id);
    } catch (e) {
      reportError(e, "Could not add the job.");
    } finally {
      setPrinting(false);
      // The second click of a double-click lands after a fast enqueue has finished; keep ignoring it.
      setTimeout(() => {
        printingRef.current = false;
      }, REPRINT_GUARD_MS);
    }
  }, [notify, printBlockedReason, printer, reportError, result, settings, ws]);

  // ---- files
  const openFile = useCallback(async () => {
    try {
      const info = await api().openSource();
      if (!info) return;
      if (info.kind === "pdf") {
        setWs((w) => ({ ...w, mode: "pdf", pdf: { ...w.pdf, info, page: 1, password: "" } }));
      } else {
        setWs((w) => ({ ...w, mode: "image", image: { ...w.image, info } }));
      }
      notify(`Opened ${info.name}.`, "info");
    } catch (e) {
      reportError(e, "Could not open that file.");
    }
  }, [notify, reportError]);

  const savePng = useCallback(async () => {
    if (!result || !preview.current) {
      notify("Wait for the preview to finish before saving it.", "info");
      return;
    }
    try {
      if (!(await api().savePreviewPng(result.snapshot_id))) return;
      notify("Saved the preview as a PNG image (without the extra feed).", "success");
    } catch (e) {
      reportError(e, "Could not save the preview.");
    }
  }, [notify, preview.current, reportError, result]);

  const value = useMemo<WorkspaceContextValue>(
    () => ({
      ws,
      setWs,
      preview,
      settings,
      setDensity,
      setFeedText,
      normalizeFeed,
      setCopiesText,
      normalizeCopies,
      paperMm,
      printBlockedReason,
      printing,
      print,
      openFile,
      savePng,
    }),
    [
      ws,
      preview,
      settings,
      setDensity,
      setFeedText,
      normalizeFeed,
      setCopiesText,
      normalizeCopies,
      paperMm,
      printBlockedReason,
      printing,
      print,
      openFile,
      savePng,
    ],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
