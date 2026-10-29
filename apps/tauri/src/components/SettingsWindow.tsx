import { useEffect, useRef, type KeyboardEvent, type ReactElement } from "react";
import { shortcut } from "../lib/format";
import { useApp, type SettingsSection } from "../state/AppStore";
import { AppearanceSettings } from "./AppearanceSettings";
import { DiagnosticsView } from "./DiagnosticsView";
import { CloseIcon, PaletteIcon, PrinterIcon, PulseIcon, UpdateIcon } from "./icons";
import { PrintersView } from "./PrintersView";
import { UpdatesSettings } from "./UpdatesSettings";

const SECTIONS: { id: SettingsSection; label: string; icon: () => ReactElement }[] = [
  { id: "printers", label: "Printers", icon: PrinterIcon },
  { id: "appearance", label: "Appearance", icon: PaletteIcon },
  { id: "updates", label: "Updates", icon: UpdateIcon },
  { id: "diagnostics", label: "Diagnostics", icon: PulseIcon },
];

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Settings, over the dimmed window (Ctrl+, or ⌘,): saved printers and finding new ones,
 * appearance, updates, and diagnostics. Modal. Escape closes it unless a dialog inside it is open; those
 * dialogs stop the key before it gets here.
 */
export function SettingsWindow({ section }: { section: SettingsSection }) {
  const { setView } = useApp();
  const ref = useRef<HTMLDivElement>(null);
  const close = () => setView("workspace");

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.querySelector<HTMLElement>('[aria-current="page"]')?.focus();
    return () => {
      if (previous?.isConnected) previous.focus();
    };
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab" && ref.current) {
      const items = [...ref.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
      const first = items[0];
      const last = items[items.length - 1];
      if (!first || !last) return;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        ref={ref}
        className="settings-window"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <nav className="settings-nav" aria-label="Settings sections">
          <h2 id="settings-title" className="settings-title" title={shortcut(",")}>
            Settings
          </h2>
          {SECTIONS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              className={`settings-tab${id === section ? " is-current" : ""}`}
              aria-current={id === section ? "page" : undefined}
              onClick={() => setView(id)}
            >
              <Icon />
              {label}
            </button>
          ))}
        </nav>
        <div className="settings-content">
          <button type="button" className="btn btn-quiet icon-only btn-small settings-close" onClick={close} aria-label="Close settings">
            <CloseIcon />
          </button>
          {section === "printers" ? <PrintersView /> : null}
          {section === "appearance" ? <AppearanceSettings /> : null}
          {section === "updates" ? <UpdatesSettings /> : null}
          {section === "diagnostics" ? <DiagnosticsView /> : null}
        </div>
      </div>
    </div>
  );
}
