import { useState } from "react";
import { formatMm } from "../../lib/estimate";
import { DENSITY_LABEL } from "../../lib/labels";
import { shortcut } from "../../lib/format";
import { useApp } from "../../state/AppStore";
import { useWorkspace } from "../../state/WorkspaceStore";
import { SearchIcon } from "../icons";
import { NumberField } from "../ui";

export function PrintPanel() {
  const { data, selectedPrinter, selectedProfile, setView, startScan } = useApp();
  const scanning = data.scan.status === "starting" || data.scan.status === "scanning";
  const {
    settings,
    setDensity,
    setFeedText,
    normalizeFeed,
    setCopiesText,
    normalizeCopies,
    preview,
    paperMm,
    printBlockedReason,
    printing,
    print,
  } = useWorkspace();

  const [feedInfoOpen, setFeedInfoOpen] = useState(false);
  const profile = selectedProfile;
  const result = preview.result;
  // A long printer name can still be cut off in the narrow summary; the tooltip has it all.
  const reason = printBlockedReason ?? (selectedPrinter ? `Prints on ${selectedPrinter.name}` : "");

  return (
    <section className="print-bar" aria-label="Print">
      {!selectedPrinter ? (
        <div className="print-bar-note">
          <p>
            <strong>No printer yet.</strong> Turn the printer on, then find and save it.
          </p>
          <button
            type="button"
            className="btn"
            onClick={() => {
              setView("printers");
              if (!scanning) void startScan(false);
            }}
          >
            <SearchIcon />
            Find printers…
          </button>
        </div>
      ) : !profile || !settings ? (
        <p className="print-bar-note note-warn">
          <strong>Unavailable: </strong>This printer's profile is not available in this version of NuPrint. Review
          it in Settings → Printers.
        </p>
      ) : (
        <div className="print-settings">
          <div className="field">
            <label htmlFor="density" className="field-label">
              Darkness
            </label>
            <select id="density" value={settings.density} onChange={(e) => setDensity(e.target.value as typeof settings.density)}>
              {profile.densities.map((d) => (
                <option key={d} value={d}>
                  {DENSITY_LABEL[d]}
                </option>
              ))}
            </select>
          </div>

          <div className="field">
            <label htmlFor="copies" className="field-label" title={`1–${profile.max_copies}`}>
              Copies
            </label>
            <NumberField
              id="copies"
              name="copies"
              min={1}
              max={profile.max_copies}
              value={settings.copiesText}
              onChange={setCopiesText}
              onBlur={normalizeCopies}
            />
          </div>

          <div className="field">
            <span
              className="field-label-row"
              onMouseEnter={() => setFeedInfoOpen(true)}
              onMouseLeave={() => setFeedInfoOpen(false)}
            >
              <label htmlFor="feed" className="field-label">
                Extra feed (mm)
              </label>
              <button
                type="button"
                className="info-btn"
                aria-label="About extra feed"
                aria-describedby="feed-info"
                onFocus={() => setFeedInfoOpen(true)}
                onBlur={() => setFeedInfoOpen(false)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setFeedInfoOpen(false);
                }}
              >
                i
              </button>
              <span id="feed-info" role="tooltip" className="popup popup-up" hidden={!feedInfoOpen}>
                Whole millimetres, {profile.feed.min_mm}–{profile.feed.max_mm}. 0 = no extra feed · Less feed may leave
                the last line inside the printer. Applied once, after the final copy.
                {!profile.feed.calibrated_reduced
                  ? " Reduced values have not been measured on real paper yet."
                  : ""}
              </span>
            </span>
            <NumberField
              id="feed"
              name="millimetres of feed"
              min={profile.feed.min_mm}
              max={profile.feed.max_mm}
              value={settings.feedText}
              onChange={setFeedText}
              onBlur={normalizeFeed}
            />
          </div>
        </div>
      )}

      <div className="print-summary">
        <p className="paper-estimate" data-testid="paper-estimate">
          {result && settings && paperMm !== null ? (
            <>
              <strong>≈ {formatMm(paperMm)} of paper</strong>
              <span>
                {settings.copies} × {result.length_mm.toFixed(1)}
                {settings.feedMm > 0 ? ` + ${settings.feedMm} mm feed` : " mm, no extra feed"}
              </span>
            </>
          ) : (
            <>
              <strong>Paper needed</strong>
              <span>Shown with the preview.</span>
            </>
          )}
        </p>
        {/* Always there, so the bar keeps its layout as the reason comes and goes. */}
        <span id="print-reason" className="hint" title={reason || undefined}>
          {reason}
        </span>
      </div>
      <button
        type="button"
        className="btn btn-primary print-btn"
        onClick={() => void print()}
        disabled={Boolean(printBlockedReason) || printing || !selectedPrinter}
        aria-describedby="print-reason"
        title={`Print (${shortcut("P")})`}
      >
        {printing ? "Adding to queue…" : "Print"}
      </button>
    </section>
  );
}
