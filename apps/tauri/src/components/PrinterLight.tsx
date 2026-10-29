import type { PrinterCheck } from "../bindings/PrinterCheck";
import type { StatusFlag } from "../bindings/StatusFlag";
import { FLAG_LABEL } from "../lib/labels";

type Light = "idle" | "busy" | "good" | "warn" | "bad";

/** Flags that do not stop a print. */
const HARMLESS: StatusFlag[] = ["charging", "printing"];

/** The light and its words for a printer's last connection check in this session. */
export function printerLight(check: PrinterCheck | undefined, checking: boolean, failed: boolean): {
  light: Light;
  label: string;
} {
  if (checking) return { light: "busy", label: "Checking…" };
  if (failed) return { light: "bad", label: "Could not connect" };
  if (!check) return { light: "idle", label: "Not checked yet" };
  const problems = check.flags.filter((f) => !HARMLESS.includes(f));
  if (check.ready && problems.length === 0) return { light: "good", label: "Ready" };
  return { light: "warn", label: problems.length > 0 ? problems.map((f) => FLAG_LABEL[f]).join(", ") : "Not ready" };
}

/** A little printer whose status light shows the last check; paper shows unless it ran out. */
export function PrinterLight({ name, check, checking, failed, decorative = false }: {
  name: string;
  check: PrinterCheck | undefined;
  checking: boolean;
  failed: boolean;
  /** When the words are written beside it: then the drawing is hidden from screen readers. */
  decorative?: boolean;
}) {
  const { light, label } = printerLight(check, checking, failed);
  const paper = !check?.flags.includes("paper_out");
  const a11y = decorative
    ? ({ "aria-hidden": true } as const)
    : ({ role: "img", "aria-label": `${name}: ${label}`, title: label } as const);
  return (
    <span className="printer-light" data-light={light} {...a11y}>
      <svg viewBox="0 0 40 40" aria-hidden="true">
        {paper ? <path className="pl-paper" d="M13 17V6l1.75-1.5L16.5 6l1.75-1.5L20 6l1.75-1.5L23.5 6l1.75-1.5L27 6v11z" /> : null}
        <rect className="pl-body" x="5" y="14" width="30" height="22" rx="8" />
        <rect className="pl-slot" x="11" y="15.5" width="18" height="2.4" rx="1.2" />
        <circle className="pl-led" cx="28.5" cy="29" r="3" />
      </svg>
    </span>
  );
}
