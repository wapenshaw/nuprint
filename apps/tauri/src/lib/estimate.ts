/**
 * Total paper estimate: every copy prints `lengthMm` of raster, and the extra feed
 * is applied once after the final copy (never per copy or per page).
 * The preview PNG excludes the feed.
 */
export function paperEstimateMm(lengthMm: number, copies: number, feedMm: number): number {
  const c = Number.isFinite(copies) ? Math.max(1, Math.floor(copies)) : 1;
  const f = Number.isFinite(feedMm) ? Math.max(0, feedMm) : 0;
  return c * lengthMm + f;
}

/** Clamp to an integer in [min, max]; non-numeric input falls back to `fallback`. */
export function clampInt(value: number | string, min: number, max: number, fallback: number): number {
  const n = typeof value === "number" ? value : Number.parseFloat(value);
  if (!Number.isFinite(n)) return Math.min(max, Math.max(min, Math.round(fallback)));
  return Math.min(max, Math.max(min, Math.round(n)));
}

/** "148 mm" / "148.5 mm": at most one decimal, no trailing zero. */
export function formatMm(mm: number): string {
  const rounded = Math.round(mm * 10) / 10;
  return `${Number.isInteger(rounded) ? rounded.toString() : rounded.toFixed(1)} mm`;
}
