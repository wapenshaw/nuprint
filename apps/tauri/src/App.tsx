import { useEffect } from "react";
import { Header } from "./components/Header";
import { QueuePanel } from "./components/QueueView";
import { SettingsWindow } from "./components/SettingsWindow";
import { StatusBar } from "./components/StatusBar";
import { ErrorNote } from "./components/ui";
import { WorkspaceView } from "./components/workspace/WorkspaceView";
import { AppProvider, useApp } from "./state/AppStore";
import { WorkspaceProvider, useWorkspace } from "./state/WorkspaceStore";

/** Ctrl/Cmd+O open, Ctrl/Cmd+P print current preview, Ctrl/Cmd+S save preview PNG, Ctrl/Cmd+, settings. */
function useShortcuts() {
  const { openFile, print, savePng } = useWorkspace();
  const { setView } = useApp();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      const key = e.key.toLowerCase();
      if (key === "o") {
        e.preventDefault();
        setView("workspace");
        void openFile();
      } else if (key === "p") {
        e.preventDefault();
        void print();
      } else if (key === "s") {
        e.preventDefault();
        void savePng();
      } else if (key === ",") {
        e.preventDefault();
        setView("printers");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openFile, print, savePng, setView]);
}

/**
 * One window: the toolbar, the workspace (what to print, its paper preview and the print bar)
 * and the status bar. The queue slides over the workspace and settings open on top; neither
 * moves anything in the window.
 */
function Shell() {
  const { data, view } = useApp();
  useShortcuts();

  return (
    <div className="app">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <Header />
      <div className="app-body">
        <main id="main" className="app-main" tabIndex={-1}>
          {data.loadError ? (
            <div className="app-message">
              <ErrorNote>Could not start NuPrint: {data.loadError.message}</ErrorNote>
            </div>
          ) : !data.ready ? (
            <p role="status" className="app-message muted">
              Loading…
            </p>
          ) : (
            <WorkspaceView />
          )}
        </main>
        {data.ready && view === "queue" ? <QueuePanel /> : null}
      </div>
      <StatusBar />
      {data.ready && view !== "workspace" && view !== "queue" ? <SettingsWindow section={view} /> : null}
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <WorkspaceProvider>
        <Shell />
      </WorkspaceProvider>
    </AppProvider>
  );
}
