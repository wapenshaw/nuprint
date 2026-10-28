import type { ImageAdjust } from "../../bindings/ImageAdjust";
import type { SourceInfo } from "../../bindings/SourceInfo";
import type { TextSpec } from "../../bindings/TextSpec";
import { needsPdfPassword } from "../../lib/errors";
import { formatBytes, shortcut } from "../../lib/format";
import { useApp } from "../../state/AppStore";
import { useWorkspace } from "../../state/WorkspaceStore";
import { DEFAULT_ADJUST, SIZE_PRESETS, type SourceMode } from "../../state/workspace";
import { OpenIcon, RotateIcon } from "../icons";
import { NumberField, Segmented, WarnNote } from "../ui";

const FAMILIES: { id: TextSpec["family"]; label: string }[] = [
  { id: "sans", label: "Sans" },
  { id: "mono", label: "Mono" },
];

const ALIGNS: { id: TextSpec["align"]; label: string }[] = [
  { id: "left", label: "Left" },
  { id: "center", label: "Center" },
  { id: "right", label: "Right" },
];

const TONES: { id: ImageAdjust["tone"]; label: string }[] = [
  { id: "dither", label: "Photo shading" },
  { id: "atkinson", label: "High contrast" },
  { id: "threshold", label: "Crisp" },
];

const MODES: { id: SourceMode; label: string }[] = [
  { id: "text", label: "Text" },
  { id: "image", label: "Image" },
  { id: "pdf", label: "PDF" },
  { id: "test_pattern", label: "Test pattern" },
];

export function SourcePanel() {
  const { ws, setWs } = useWorkspace();
  return (
    <section className="inspector" aria-labelledby="source-heading">
      <h2 id="source-heading">What to print</h2>
      <Segmented
        label="Source type"
        hideLabel
        fill
        value={ws.mode}
        options={MODES}
        onChange={(mode) => setWs((w) => ({ ...w, mode }))}
      />

      {ws.mode === "text" ? <TextControls /> : null}
      {ws.mode === "image" ? <ImageControls /> : null}
      {ws.mode === "pdf" ? <PdfControls /> : null}
      {ws.mode === "test_pattern" ? (
        <p className="muted controls">
          A small calibration strip: a border, bars and the printable width. Use it to check alignment and
          darkness before printing real content.
        </p>
      ) : null}
    </section>
  );
}

function TextControls() {
  const { ws, setWs } = useWorkspace();
  const t = ws.text;
  const set = (patch: Partial<TextSpec>) => setWs((w) => ({ ...w, text: { ...w.text, ...patch } }));
  const sizeOptions = SIZE_PRESETS.includes(t.size_px as (typeof SIZE_PRESETS)[number])
    ? [...SIZE_PRESETS]
    : [...SIZE_PRESETS, t.size_px].sort((a, b) => a - b);

  return (
    <div className="controls">
      <div className="field">
        <label htmlFor="text-input" className="field-label">
          Text
        </label>
        <textarea
          id="text-input"
          className="text-input"
          rows={6}
          value={t.text}
          spellCheck
          onChange={(e) => set({ text: e.target.value })}
        />
      </div>

      <div className="grid-2">
        <Segmented label="Font" fill value={t.family} options={FAMILIES} onChange={(family) => set({ family })} />
        <div className="field">
          <label htmlFor="text-size" className="field-label">
            Size (px)
          </label>
          <select id="text-size" value={t.size_px} onChange={(e) => set({ size_px: Number(e.target.value) })}>
            {sizeOptions.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        </div>
      </div>

      <Segmented label="Alignment" fill value={t.align} options={ALIGNS} onChange={(align) => set({ align })} />

      <div className="grid-2">
        <div className="field">
          <label htmlFor="text-margin" className="field-label">
            Margin (dots)
          </label>
          <NumberField
            id="text-margin"
            name="margin dots"
            min={0}
            max={96}
            value={String(t.margin_dots)}
            onChange={(v) => set({ margin_dots: Math.max(0, Math.min(96, Math.round(Number(v) || 0))) })}
          />
        </div>
        <div className="field">
          <label htmlFor="text-spacing" className="field-label">
            Line spacing <output htmlFor="text-spacing">{t.line_spacing.toFixed(1)}×</output>
          </label>
          <input
            id="text-spacing"
            type="range"
            min={0.8}
            max={3}
            step={0.1}
            value={t.line_spacing}
            onChange={(e) => set({ line_spacing: Number(e.target.value) })}
          />
        </div>
      </div>

      <div className="checks">
        <label className="check">
          <input type="checkbox" checked={t.bold} onChange={(e) => set({ bold: e.target.checked })} />
          Bold
        </label>
        <label className="check">
          <input type="checkbox" checked={t.headings} onChange={(e) => set({ headings: e.target.checked })} />
          Lines starting with “# ” are headings
        </label>
      </div>
    </div>
  );
}

function FileSummary({ info }: { info: SourceInfo }) {
  return (
    <p className="file-summary">
      {/* One line each, however long the name: the full name is in the tooltip. */}
      <strong title={info.name}>{info.name}</strong>
      <span className="muted">
        {formatBytes(info.bytes)}
        {info.page_count ? ` · ${info.page_count} page${info.page_count === 1 ? "" : "s"}` : ""}
      </span>
    </p>
  );
}

function OpenButton({ label }: { label: string }) {
  const { openFile } = useWorkspace();
  return (
    <button type="button" className="btn" onClick={() => void openFile()}>
      <OpenIcon />
      {label}
    </button>
  );
}

function ImageControls() {
  const { ws, setWs } = useWorkspace();
  const { info, adjust } = ws.image;
  return (
    <div className="controls">
      <div className="row">
        <OpenButton label="Open…" />
        <span className="hint">PNG, JPEG, BMP, GIF, WebP or TIFF · {shortcut("O")}</span>
      </div>
      {info ? <FileSummary info={info} /> : <p className="muted">No image opened yet.</p>}
      {info ? (
        <AdjustControls
          adjust={adjust}
          onChange={(a) => setWs((w) => ({ ...w, image: { ...w.image, adjust: a } }))}
          idPrefix="img"
        />
      ) : null}
    </div>
  );
}

function PdfControls() {
  const { ws, setWs, preview } = useWorkspace();
  const { data } = useApp();
  const { info, adjust, page, password } = ws.pdf;
  const askPassword = password !== "" || (preview.error ? needsPdfPassword(preview.error) : false);
  const pages = info?.page_count ?? 1;
  return (
    <div className="controls">
      {data.info && !data.info.pdf_available ? (
        <WarnNote>PDF support is not available on this computer. {data.info.pdf_error ?? ""}</WarnNote>
      ) : null}
      <div className="row">
        <OpenButton label="Open…" />
        <span className="hint">One PDF page per print · {shortcut("O")}</span>
      </div>
      {info ? <FileSummary info={info} /> : <p className="muted">No PDF opened yet.</p>}
      {info ? (
        <>
          <div className="field">
            <label htmlFor="pdf-page" className="field-label">
              Page <span className="muted">of {pages}</span>
            </label>
            <NumberField
              id="pdf-page"
              name="pages"
              min={1}
              max={pages}
              value={String(page)}
              onChange={(v) => {
                const n = Math.round(Number(v));
                if (Number.isFinite(n)) setWs((w) => ({ ...w, pdf: { ...w.pdf, page: Math.max(1, Math.min(pages, n)) } }));
              }}
            />
          </div>
          {askPassword ? (
            <div className="field">
              <label htmlFor="pdf-password" className="field-label">
                PDF password
              </label>
              <input
                id="pdf-password"
                type="password"
                autoComplete="off"
                value={password}
                onChange={(e) => {
                  const v = e.target.value;
                  setWs((w) => ({ ...w, pdf: { ...w.pdf, password: v } }));
                }}
              />
              <span className="hint">Kept in memory only; never saved.</span>
            </div>
          ) : null}
          <AdjustControls
            adjust={adjust}
            onChange={(a) => setWs((w) => ({ ...w, pdf: { ...w.pdf, adjust: a } }))}
            idPrefix="pdf"
          />
        </>
      ) : null}
    </div>
  );
}

const ROTATIONS: ImageAdjust["rotation"][] = ["0", "90", "180", "270"];

function AdjustControls({
  adjust,
  onChange,
  idPrefix,
}: {
  adjust: ImageAdjust;
  onChange: (a: ImageAdjust) => void;
  idPrefix: string;
}) {
  const rotate = () => onChange({ ...adjust, rotation: ROTATIONS[(ROTATIONS.indexOf(adjust.rotation) + 1) % 4]! });
  return (
    <>
      <Segmented
        label="Tone"
        fill
        value={adjust.tone}
        options={TONES}
        onChange={(tone) => onChange({ ...adjust, tone })}
      />

      <div className="grid-2">
        <Slider
          id={`${idPrefix}-brightness`}
          label="Brightness"
          value={adjust.brightness}
          onChange={(v) => onChange({ ...adjust, brightness: v })}
        />
        <Slider
          id={`${idPrefix}-contrast`}
          label="Contrast"
          value={adjust.contrast}
          onChange={(v) => onChange({ ...adjust, contrast: v })}
        />
      </div>

      <div className="row adjust-row">
        <button type="button" className="btn" onClick={rotate}>
          <RotateIcon />
          Rotate 90°
        </button>
        <span className="hint">Now {adjust.rotation}°</span>
        <button
          type="button"
          className="btn btn-quiet btn-small adjust-reset"
          onClick={() => onChange({ ...DEFAULT_ADJUST, tone: adjust.tone })}
          disabled={adjust.rotation === "0" && adjust.brightness === 0 && adjust.contrast === 0}
          aria-label="Reset adjustments"
          title="Undo rotation, brightness and contrast"
        >
          Reset
        </button>
      </div>
    </>
  );
}

function Slider({ id, label, value, onChange }: { id: string; label: string; value: number; onChange: (v: number) => void }) {
  return (
    <div className="field">
      <label htmlFor={id} className="field-label">
        {label} <output htmlFor={id}>{value > 0 ? `+${value}` : value}</output>
      </label>
      <input
        id={id}
        type="range"
        min={-100}
        max={100}
        step={1}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}
