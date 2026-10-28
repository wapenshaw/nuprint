import { SUPPORT_LABEL, SUPPORT_TONE } from "../lib/labels";
import { useApp } from "../state/AppStore";
import { ChevronDownIcon, RefreshIcon, SearchIcon, SettingsIcon } from "./icons";
import { PrinterLight, printerLight } from "./PrinterLight";
import { Badge } from "./ui";
import { usePopover } from "./usePopover";

/**
 * The toolbar's printer button: the printer jobs go to and its last connection check. Its menu
 * switches printers, checks the connection and opens printer settings.
 */
export function PrinterMenu() {
  const { data, selectedPrinter, selectPrinter, setView, startScan, checking, checkErrors, runCheck } = useApp();
  const { open, setOpen, root, button } = usePopover();
  const status = (id: string) => printerLight(data.checks[id], checking === id, Boolean(checkErrors[id]));
  const current = selectedPrinter ? status(selectedPrinter.id) : null;
  const scanning = data.scan.status === "starting" || data.scan.status === "scanning";

  const manage = () => {
    setOpen(false);
    setView("printers");
  };

  return (
    <div className="printer-menu" ref={root}>
      <button
        ref={button}
        type="button"
        className="btn printer-btn"
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        {selectedPrinter && current ? (
          <>
            <PrinterLight
              name={selectedPrinter.name}
              check={data.checks[selectedPrinter.id]}
              checking={checking === selectedPrinter.id}
              failed={Boolean(checkErrors[selectedPrinter.id])}
              decorative
            />
            <span className="printer-btn-name">{selectedPrinter.name}</span>
            <span className={`printer-btn-status light-${current.light}`}>{current.label}</span>
            {SUPPORT_TONE[selectedPrinter.support] !== "good" ? (
              <Badge tone={SUPPORT_TONE[selectedPrinter.support]}>{SUPPORT_LABEL[selectedPrinter.support]}</Badge>
            ) : null}
          </>
        ) : (
          <span className="printer-btn-name muted">No printer</span>
        )}
        <ChevronDownIcon />
      </button>

      {open ? (
        <div className="menu-card printer-menu-card">
          {data.printers.length > 0 ? (
            <fieldset>
              <legend className="field-label">Print to</legend>
              {data.printers.map((p) => {
                const s = status(p.id);
                return (
                  <label key={p.id} className={`menu-option${p.id === selectedPrinter?.id ? " is-on" : ""}`}>
                    <input
                      type="radio"
                      name="printer"
                      value={p.id}
                      checked={p.id === selectedPrinter?.id}
                      onChange={() => void selectPrinter(p.id)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") setOpen(false);
                      }}
                    />
                    <PrinterLight
                      name={p.name}
                      check={data.checks[p.id]}
                      checking={checking === p.id}
                      failed={Boolean(checkErrors[p.id])}
                      decorative
                    />
                    <span className="menu-option-text">
                      <span className="menu-option-name">{p.name}</span>
                      <span className={`menu-option-sub light-${s.light}`}>
                        {s.label} · {SUPPORT_LABEL[p.support]}
                      </span>
                    </span>
                  </label>
                );
              })}
            </fieldset>
          ) : (
            <p className="menu-empty">No printer saved yet. Turn the printer on, then find it.</p>
          )}
          <div className="menu-sep" role="separator" />
          <button
            type="button"
            className="menu-item"
            disabled={!selectedPrinter || checking !== null}
            onClick={() => selectedPrinter && void runCheck(selectedPrinter.id)}
          >
            <RefreshIcon />
            Check connection
            <span className="menu-item-hint">Does not print</span>
          </button>
          <button
            type="button"
            className="menu-item"
            onClick={() => {
              manage();
              if (!scanning) void startScan(false);
            }}
          >
            <SearchIcon />
            Find printers…
          </button>
          <button type="button" className="menu-item" onClick={manage}>
            <SettingsIcon />
            Manage printers…
          </button>
        </div>
      ) : null}
    </div>
  );
}
