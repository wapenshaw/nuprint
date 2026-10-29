import { useEffect, useId, useRef, type ReactNode } from "react";
import type { SupportLevel } from "../bindings/SupportLevel";
import { SUPPORT_LABEL, SUPPORT_TONE, type BadgeTone } from "../lib/labels";
import { MinusIcon, PlusIcon } from "./icons";

export function Badge({ tone = "neutral", children, title }: { tone?: BadgeTone; children: ReactNode; title?: string }) {
  return (
    <span className={`badge badge-${tone}`} title={title}>
      {children}
    </span>
  );
}

export function SupportBadge({ level }: { level: SupportLevel }) {
  return <Badge tone={SUPPORT_TONE[level]}>{SUPPORT_LABEL[level]}</Badge>;
}

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Minimal accessible modal: labelled dialog, focus moves in and is restored on close,
 * Tab is kept inside, Escape and a click on the backdrop dismiss it.
 */
export function Modal({
  title,
  onClose,
  children,
  footer,
  wide,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const node = ref.current;
    const first = node?.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? node)?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key === "Tab" && node) {
        const items = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)];
        if (items.length === 0) return;
        const firstEl = items[0]!;
        const lastEl = items[items.length - 1]!;
        if (e.shiftKey && document.activeElement === firstEl) {
          e.preventDefault();
          lastEl.focus();
        } else if (!e.shiftKey && document.activeElement === lastEl) {
          e.preventDefault();
          firstEl.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      previous?.focus?.();
    };
  }, []);

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        className={`modal${wide ? " modal-wide" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
      >
        <div className="modal-head">
          <h2 id={titleId}>{title}</h2>
          <button type="button" className="btn btn-quiet" onClick={onClose} aria-label="Close dialog">
            Close
          </button>
        </div>
        <div className="modal-body">{children}</div>
        {footer ? <div className="modal-foot">{footer}</div> : null}
      </div>
    </div>
  );
}

export function ErrorNote({ children }: { children: ReactNode }) {
  return (
    <p className="note note-error" role="alert">
      <strong>Error: </strong>
      {children}
    </p>
  );
}

export function WarnNote({ children }: { children: ReactNode }) {
  return (
    <p className="note note-warn">
      <strong>Note: </strong>
      {children}
    </p>
  );
}

/**
 * The one switch used across the app: a track of options, one line each, all the same height.
 * Radio inputs underneath, so arrow keys move the choice and screen readers hear a group.
 */
export function Segmented<T extends string>({
  label,
  hideLabel = false,
  value,
  options,
  onChange,
  fill = false,
}: {
  label: string;
  hideLabel?: boolean;
  value: T;
  options: readonly { id: T; label: string; title?: string }[];
  onChange: (value: T) => void;
  /** Stretch across the container instead of hugging the options. */
  fill?: boolean;
}) {
  const name = useId();
  return (
    <fieldset className="segmented-field">
      <legend className={hideLabel ? "sr-only" : "field-label"}>{label}</legend>
      <div className={`segmented${fill ? " segmented-fill" : ""}`}>
        {options.map((o) => (
          <label key={o.id} className={`segment${o.id === value ? " is-on" : ""}`} title={o.title}>
            <input type="radio" name={name} value={o.id} checked={o.id === value} onChange={() => onChange(o.id)} />
            <span data-label={o.label}>{o.label}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}

/**
 * A whole-number field with − and + either side, drawn the same on every platform (the native
 * spinners differ between WebView2, WebKitGTK and WKWebView). The text stays editable; the
 * owner clamps it. The buttons stay out of the Tab order: the arrow keys do the same.
 */
export function NumberField({
  id,
  value,
  onChange,
  onBlur,
  min,
  max,
  name,
  describedBy,
}: {
  id: string;
  value: string;
  onChange: (text: string) => void;
  onBlur?: () => void;
  min: number;
  max: number;
  /** What the number counts, for the buttons' names ("copies"). */
  name: string;
  describedBy?: string;
}) {
  const current = Math.round(Number(value));
  const base = Number.isFinite(current) && value.trim() !== "" ? current : min;
  const step = (delta: number) => onChange(String(Math.max(min, Math.min(max, base + delta))));
  return (
    <div className="number-field">
      <button
        type="button"
        tabIndex={-1}
        aria-label={`Fewer ${name}`}
        disabled={base <= min}
        onClick={() => step(-1)}
      >
        <MinusIcon />
      </button>
      <input
        id={id}
        type="number"
        inputMode="numeric"
        min={min}
        max={max}
        step={1}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
        aria-describedby={describedBy}
      />
      <button
        type="button"
        tabIndex={-1}
        aria-label={`More ${name}`}
        disabled={base >= max}
        onClick={() => step(1)}
      >
        <PlusIcon />
      </button>
    </div>
  );
}
