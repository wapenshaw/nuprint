import { shortcut } from "../lib/format";
import { useApp } from "../state/AppStore";
import { BrandMark } from "./BrandMark";
import { QueueIcon, SettingsIcon } from "./icons";
import { PrinterMenu } from "./PrinterMenu";

/** The toolbar: which printer to use on the left, the queue and settings on the right. */
export function Header() {
  const { view, setView, openJobCount, appIcon } = useApp();
  const queueOpen = view === "queue";
  const settingsHint = `Settings (${shortcut(",")})`;

  return (
    <header className="app-header">
      <div className="brand">
        <BrandMark icon={appIcon} />
        <span className="brand-name">NuPrint</span>
      </div>

      <PrinterMenu />

      <div className="header-tools">
        <button
          type="button"
          className={`btn tool-btn${queueOpen ? " is-on" : ""}`}
          aria-expanded={queueOpen}
          aria-controls="queue-panel"
          onClick={() => setView(queueOpen ? "workspace" : "queue")}
        >
          <QueueIcon />
          Queue
          {openJobCount > 0 ? (
            <span className="count-badge" aria-label={`${openJobCount} job${openJobCount === 1 ? "" : "s"} in progress`}>
              {openJobCount}
            </span>
          ) : null}
        </button>
        <button
          type="button"
          className="btn tool-btn icon-only"
          aria-label="Settings"
          title={settingsHint}
          onClick={() => setView("printers")}
        >
          <SettingsIcon />
        </button>
      </div>
    </header>
  );
}
