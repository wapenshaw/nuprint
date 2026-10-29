export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatDateTime(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === today.toDateString()) return `Today ${time}`;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

export function formatRssi(rssi?: number): string {
  if (rssi === undefined || rssi === null) return "Signal unknown";
  const strength = rssi >= -60 ? "strong" : rssi >= -75 ? "fair" : "weak";
  return `${rssi} dBm · ${strength}`;
}

/** First non-empty line of the text, trimmed and shortened, as a job title. */
export function titleFromText(text: string): string {
  const line = text
    .split(/\r?\n/)
    .map((l) => l.replace(/^#+\s*/, "").trim())
    .find((l) => l.length > 0);
  if (!line) return "Text note";
  return line.length > 60 ? `${line.slice(0, 57)}…` : line;
}

export function pluralize(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** True on macOS, where shortcuts use ⌘ rather than Ctrl. */
export const IS_MAC = typeof navigator !== "undefined" && /Mac/.test(navigator.platform || navigator.userAgent);

/** A Ctrl/Cmd shortcut as the platform writes it: "⌘O" on macOS, "Ctrl+O" on Windows and Linux. */
export function shortcut(key: string): string {
  return IS_MAC ? `⌘${key}` : `Ctrl+${key}`;
}
