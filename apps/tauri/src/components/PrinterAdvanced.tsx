import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { PrinterProfileDoc } from "../bindings/PrinterProfileDoc";
import type { SavedPrinter } from "../bindings/SavedPrinter";
import { toAppError } from "../lib/errors";
import { useApp } from "../state/AppStore";
import { ErrorNote, Modal, WarnNote } from "./ui";

type Json = Record<string, unknown>;

type Range = [min: number, max: number];

/**
 * The settings people usually tune while testing; everything else is in the JSON below.
 * `range` mirrors the limits Rust enforces (docs/printer-support.md), so a value that would
 * be refused is flagged as it is typed. Rust stays the authority and re-checks on save.
 */
const QUICK_FIELDS: { label: string; path: string[]; range: (base: Json | null) => Range }[] = [
  { label: "Heat: light", path: ["driver", "energy", "light"], range: heatRange },
  { label: "Heat: normal", path: ["driver", "energy", "normal"], range: heatRange },
  { label: "Heat: dark", path: ["driver", "energy", "dark"], range: heatRange },
  { label: "Print speed", path: ["driver", "print_speed"], range: () => [1, 255] },
  { label: "Chunk size (bytes)", path: ["transport", "max_chunk_bytes"], range: () => [20, 512] },
  { label: "Delay between chunks (ms)", path: ["transport", "pacing", "chunk_delay_ms"], range: () => [0, 120_000] },
];

/** A quarter to twice the built-in normal heat. */
function heatRange(base: Json | null): Range {
  const normal = base ? getPath(base, ["driver", "energy", "normal"]) : undefined;
  const n = typeof normal === "number" ? normal : 5000;
  return [Math.floor(n / 4), n * 2];
}

/** A short horizontal shake; skipped when the person asked for less motion. */
function shake(el: Element | null) {
  if (!el || typeof el.animate !== "function") return;
  if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
  el.animate(
    [
      { transform: "translateX(0)" },
      { transform: "translateX(-6px)" },
      { transform: "translateX(6px)" },
      { transform: "translateX(-4px)" },
      { transform: "translateX(4px)" },
      { transform: "translateX(0)" },
    ],
    { duration: 320, easing: "ease-in-out" },
  );
}

function parse(text: string): Json | null {
  try {
    const value: unknown = JSON.parse(text);
    return value && typeof value === "object" && !Array.isArray(value) ? (value as Json) : null;
  } catch {
    return null;
  }
}

function getPath(doc: Json, path: string[]): unknown {
  return path.reduce<unknown>((node, key) => (node && typeof node === "object" ? (node as Json)[key] : undefined), doc);
}

function setPath(doc: Json, path: string[], value: number): Json {
  const [head, ...rest] = path;
  if (head === undefined) return doc;
  const child = rest.length === 0 ? value : setPath(((doc[head] as Json | undefined) ?? {}) as Json, rest, value);
  return { ...doc, [head]: child };
}

/**
 * Edit every tunable setting of one saved printer (Printers → Advanced). The built-in
 * profile is never modified: edits become this printer's own settings, and Reset returns to
 * the built-in ones. Rust validates everything and keeps the printer's identity fixed.
 */
export function PrinterAdvanced({ printer, onClose }: { printer: SavedPrinter; onClose: () => void }) {
  const { refreshPrinters, notify, reportError } = useApp();
  const [doc, setDoc] = useState<PrinterProfileDoc | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** What a quick field shows while it is being typed in, e.g. while it is empty. */
  const [typing, setTyping] = useState<Record<string, string>>({});
  const errorRef = useRef<HTMLDivElement>(null);

  // A refusal from Rust must be noticed even when the dialog is scrolled away from it.
  useEffect(() => {
    if (!error) return;
    errorRef.current?.scrollIntoView?.({ block: "nearest" });
    shake(errorRef.current);
  }, [error]);

  useEffect(() => {
    let alive = true;
    api()
      .getPrinterProfile(printer.id)
      .then((d) => {
        if (!alive) return;
        setDoc(d);
        setText(d.current_json);
      })
      .catch((e: unknown) => alive && setError(toAppError(e).message));
    return () => {
      alive = false;
    };
  }, [printer.id]);

  const parsed = parse(text);
  const base = doc ? parse(doc.base_json) : null;
  const changed = doc !== null && text !== doc.current_json;
  const outOfRange = (id: string, value: unknown, range: Range) => {
    const shown = typing[id] ?? value;
    const n = typeof shown === "string" ? (shown === "" ? NaN : Number(shown)) : shown;
    return typeof n !== "number" ? false : !Number.isInteger(n) || n < range[0] || n > range[1];
  };
  const anyInvalid = QUICK_FIELDS.some((f) =>
    outOfRange(`adv-${f.path.join("-")}`, parsed ? getPath(parsed, f.path) : undefined, f.range(base)),
  );

  async function run(action: () => Promise<void>, failure: string) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(toAppError(e).message || failure);
    } finally {
      setBusy(false);
    }
  }

  const save = () =>
    run(async () => {
      await api().savePrinterProfile({ printer_id: printer.id, json: text });
      await refreshPrinters();
      notify(
        `Saved the settings for ${printer.name}. Print a short test strip before relying on them.`,
        "success",
      );
      onClose();
    }, "Could not save the settings.");

  const reset = () =>
    run(async () => {
      await api().resetPrinterProfile(printer.id);
      await refreshPrinters();
      notify(`${printer.name} uses its built-in settings again.`, "success");
      onClose();
    }, "Could not reset the settings.");

  const exportFile = async () => {
    try {
      if (await api().exportPrinterProfile(printer.id)) notify("Saved the printer profile file.", "success");
    } catch (e) {
      reportError(e, "Could not export the profile.");
    }
  };

  return (
    <Modal
      title={`Advanced settings · ${printer.name}`}
      onClose={onClose}
      wide
      footer={
        <>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => void exportFile()}
            disabled={busy || !doc}
            title="Writes the saved settings as a complete profile file you can keep or share"
          >
            Export profile…
          </button>
          <button type="button" className="btn" onClick={() => void reset()} disabled={busy || !doc?.customized}>
            Reset to defaults
          </button>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void save()}
            disabled={busy || !changed || parsed === null || anyInvalid}
          >
            Save settings
          </button>
        </>
      }
    >
      <WarnNote>
        These values go straight to the printer. A wrong value can make printing fail, come out garbled, too light or
        too dark, or waste paper. The settings apply only to this printer on this computer, and edited settings are
        always shown as experimental. What the printer is called and how it is recognised cannot be changed.
      </WarnNote>

      {doc ? (
        <>
          <p className="hint">
            Based on the built-in <strong>{doc.base_display_name}</strong> profile
            {doc.customized ? ", with your edits." : "."} Test one short strip after each change; save when it prints
            well.
          </p>

          <div className="quick-fields" role="group" aria-label="Common settings">
            {QUICK_FIELDS.map((f) => {
              const value = parsed ? getPath(parsed, f.path) : undefined;
              const id = `adv-${f.path.join("-")}`;
              const range = f.range(base);
              const invalid = outOfRange(id, value, range);
              return (
                <div className="field" key={id}>
                  <label htmlFor={id} className="field-label">
                    {f.label}
                  </label>
                  <input
                    id={id}
                    type="number"
                    className="narrow"
                    inputMode="numeric"
                    min={0}
                    step={1}
                    disabled={typeof value !== "number"}
                    value={typing[id] ?? (typeof value === "number" ? value : "")}
                    aria-invalid={invalid}
                    aria-describedby={invalid ? `${id}-range` : undefined}
                    onChange={(e) => {
                      const raw = e.target.value;
                      setTyping((t) => ({ ...t, [id]: raw }));
                      if (parsed && raw !== "" && Number.isFinite(Number(raw))) {
                        setText(JSON.stringify(setPath(parsed, f.path, Number(raw)), null, 2));
                      }
                      // Too high can only get worse with more digits; too low may be mid-typing.
                      if (Number(raw) > range[1]) shake(e.target);
                    }}
                    onBlur={(e) => {
                      if (invalid) shake(e.target);
                      setTyping((t) => {
                        const { [id]: _done, ...rest } = t;
                        return rest;
                      });
                    }}
                  />
                  {invalid ? (
                    <span id={`${id}-range`} className="field-error" role="alert">
                      Allowed: {range[0]}–{range[1]}
                    </span>
                  ) : null}
                </div>
              );
            })}
          </div>

          <div className="field">
            <label htmlFor="adv-json" className="field-label">
              All editable settings (JSON)
            </label>
            <textarea
              id="adv-json"
              className="json-editor"
              rows={14}
              spellCheck={false}
              value={text}
              onChange={(e) => setText(e.target.value)}
              aria-invalid={parsed === null}
            />
            {parsed === null ? <span className="hint">This is not a valid JSON object yet.</span> : null}
          </div>
        </>
      ) : error ? null : (
        <p role="status">Loading settings…</p>
      )}

      {error ? (
        <div ref={errorRef}>
          <ErrorNote>{error}</ErrorNote>
        </div>
      ) : null}
    </Modal>
  );
}
