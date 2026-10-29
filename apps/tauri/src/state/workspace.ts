import type { ImageAdjust } from "../bindings/ImageAdjust";
import type { SourceInfo } from "../bindings/SourceInfo";
import type { SourceSpec } from "../bindings/SourceSpec";
import type { TextSpec } from "../bindings/TextSpec";
import { titleFromText } from "../lib/format";

export type SourceMode = "text" | "image" | "pdf" | "test_pattern";

export const SAMPLE_TEXT = "Hello from NuPrint!\n\nYour printer is ready.";

export const SIZE_PRESETS = [12, 16, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56, 64, 72, 96] as const;

export const DEFAULT_TEXT: TextSpec = {
  text: SAMPLE_TEXT,
  size_px: 28,
  family: "sans",
  bold: false,
  align: "left",
  line_spacing: 1.2,
  margin_dots: 8,
  headings: true,
};

export const DEFAULT_ADJUST: ImageAdjust = { tone: "dither", rotation: "0", brightness: 0, contrast: 0 };

export interface ImageState {
  info: SourceInfo | null;
  adjust: ImageAdjust;
}

export interface PdfState {
  info: SourceInfo | null;
  page: number;
  adjust: ImageAdjust;
  password: string;
}

export interface WorkspaceState {
  mode: SourceMode;
  text: TextSpec;
  image: ImageState;
  pdf: PdfState;
}

export const initialWorkspace: WorkspaceState = {
  mode: "text",
  text: DEFAULT_TEXT,
  image: { info: null, adjust: DEFAULT_ADJUST },
  pdf: { info: null, page: 1, adjust: { ...DEFAULT_ADJUST, tone: "threshold" }, password: "" },
};

/** The source description sent to the renderer, or null when there is nothing to render yet. */
export function buildSourceSpec(ws: WorkspaceState): SourceSpec | null {
  switch (ws.mode) {
    case "text":
      return ws.text.text.trim().length === 0 ? null : { kind: "text", ...ws.text };
    case "image":
      return ws.image.info ? { kind: "image", source_id: ws.image.info.source_id, adjust: ws.image.adjust } : null;
    case "pdf": {
      if (!ws.pdf.info) return null;
      const spec: SourceSpec = {
        kind: "pdf",
        source_id: ws.pdf.info.source_id,
        page: ws.pdf.page,
        adjust: ws.pdf.adjust,
      };
      if (ws.pdf.password) spec.password = ws.pdf.password;
      return spec;
    }
    case "test_pattern":
      return { kind: "test_pattern" };
  }
}

export function jobTitle(ws: WorkspaceState): string {
  switch (ws.mode) {
    case "text":
      return titleFromText(ws.text.text);
    case "image":
      return ws.image.info?.name ?? "Image";
    case "pdf":
      return ws.pdf.info ? `${ws.pdf.info.name} · page ${ws.pdf.page}` : "PDF";
    case "test_pattern":
      return "Test pattern";
  }
}

export function emptyHint(mode: SourceMode): string {
  switch (mode) {
    case "text":
      return "Type some text to see the preview.";
    case "image":
      return "Open an image to see the preview.";
    case "pdf":
      return "Open a PDF to see the preview.";
    case "test_pattern":
      return "";
  }
}
