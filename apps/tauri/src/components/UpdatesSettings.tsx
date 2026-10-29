import { useState } from "react";
import { api } from "../api";
import { errorMessage } from "../lib/errors";
import { isActive } from "../lib/labels";
import { loadCheckAtStart, saveCheckAtStart } from "../lib/updates";
import { useApp } from "../state/AppStore";
import { ErrorNote } from "./ui";

/** Check for a newer NuPrint and install it. Nothing is checked unless the user asks. */
export function UpdatesSettings() {
  const { data, update, setUpdate, installingUpdate: installing, installUpdate } = useApp();
  const [checkAtStart, setCheckAtStart] = useState(loadCheckAtStart);
  const [checking, setChecking] = useState(false);
  const [upToDate, setUpToDate] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const printing = data.jobs.some((j) => isActive(j.state));
  const linux = data.info?.os.startsWith("linux") ?? false;

  async function check() {
    setChecking(true);
    setError(null);
    setUpToDate(false);
    try {
      const found = await api().checkForUpdate();
      setUpdate(found);
      setUpToDate(!found);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setChecking(false);
    }
  }

  return (
    <div className="settings-page">
      <div className="settings-head">
        <h2>Updates</h2>
      </div>

      <section className="card" aria-labelledby="version-heading">
        <h3 id="version-heading">NuPrint {data.info?.version}</h3>
        <div className="row">
          <button type="button" className="btn" onClick={() => void check()} disabled={checking || installing}>
            {checking ? "Checking…" : "Check for updates"}
          </button>
          {upToDate ? (
            <span className="muted" role="status">
              This is the newest version.
            </span>
          ) : null}
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={checkAtStart}
            onChange={(e) => {
              saveCheckAtStart(e.target.checked);
              setCheckAtStart(e.target.checked);
            }}
          />
          Check for updates when NuPrint starts
        </label>
        <p className="hint">
          A check asks GitHub, where NuPrint is published, for the newest release. Nothing about you, your printers
          or your documents is sent.
        </p>
      </section>

      {update ? (
        <section className="card" aria-labelledby="update-heading">
          <h3 id="update-heading">NuPrint {update.version} is available</h3>
          {update.notes ? (
            <p className="update-notes" tabIndex={0} aria-label="What’s new">
              {update.notes}
            </p>
          ) : null}
          <div className="row">
            {update.can_install ? (
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => {
                  setError(null);
                  void installUpdate();
                }}
                disabled={printing || installing}
              >
                {installing ? "Installing…" : "Install and restart"}
              </button>
            ) : null}
            <button
              type="button"
              className="btn"
              onClick={() => api().openReleasePage().catch((e: unknown) => setError(errorMessage(e)))}
            >
              {update.can_install ? "Release page" : "Download"}
            </button>
          </div>
          <p className="hint">
            {printing
              ? "A job is printing. Install the update when it has finished."
              : `NuPrint closes, installs the update and opens again. Waiting jobs are kept.${
                  linux ? " Linux asks for your password to install it." : ""
                }`}
          </p>
        </section>
      ) : null}

      {error ? <ErrorNote>{error}</ErrorNote> : null}
    </div>
  );
}
