import { describe, expect, it } from "vitest";
import type { LogLine } from "../bindings/LogLine";
import { MAX_LOG_LINES, appendLog } from "./reducer";

const line = (seq: number): LogLine => ({ seq, at_ms: 0, level: "info", message: `line ${seq}` });
const seqs = (logs: LogLine[]) => logs.map((l) => l.seq);

describe("appendLog", () => {
  it("keeps a line delivered after a newer one, in sequence order", () => {
    // Sequence 2 was assigned first but its event arrived second.
    expect(seqs(appendLog(appendLog([line(1)], line(3)), line(2)))).toEqual([1, 2, 3]);
  });

  it("skips a line already in the log", () => {
    const logs = [line(1), line(2)];
    expect(appendLog(logs, line(1))).toBe(logs);
    expect(appendLog(logs, line(2))).toBe(logs);
  });

  it("keeps only the newest lines", () => {
    const full = Array.from({ length: MAX_LOG_LINES }, (_, i) => line(i + 1));
    const next = appendLog(full, line(MAX_LOG_LINES + 1));
    expect(next).toHaveLength(MAX_LOG_LINES);
    expect(next[0]!.seq).toBe(2);
    expect(next.at(-1)!.seq).toBe(MAX_LOG_LINES + 1);
  });
});
