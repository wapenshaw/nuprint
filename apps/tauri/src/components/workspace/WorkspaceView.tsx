import { PreviewPane } from "./PreviewPane";
import { PrintPanel } from "./PrintPanel";
import { SourcePanel } from "./SourcePanel";

/** What to print on the left; on the right its paper preview and, under it, the print bar. */
export function WorkspaceView() {
  return (
    <div className="workspace">
      <SourcePanel />
      <div className="workspace-right">
        <PreviewPane />
        <PrintPanel />
      </div>
    </div>
  );
}
