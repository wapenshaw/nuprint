import type { ReactNode } from "react";
import type { AppIconId } from "../lib/theme";

/**
 * The header logo: a one-colour version of the chosen app icon, drawn in the accent ink on
 * the accent colour so it follows the accent theme.
 */
export function BrandMark({ icon }: { icon: AppIconId }) {
  return (
    <span className="brand-mark" aria-hidden="true">
      <svg viewBox="0 0 24 24">{GLYPHS[icon]}</svg>
    </span>
  );
}

const INK = "var(--accent-ink)";
const ACCENT = "var(--accent)";

const GLYPHS: Record<AppIconId, ReactNode> = {
  cube: (
    <>
      <path d="M8.6 10.5V5.2l1.13-1 1.13 1 1.14-1 1.13 1 1.14-1 1.13 1v5.3z" fill={INK} fillOpacity={0.72} />
      <rect x="5" y="9.6" width="14" height="10.4" rx="3.6" fill={INK} />
      <circle cx="10" cy="14.3" r="1" fill={ACCENT} />
      <circle cx="14" cy="14.3" r="1" fill={ACCENT} />
      <path d="M10.6 16.5Q12 17.6 13.4 16.5" fill="none" stroke={ACCENT} strokeWidth={0.9} strokeLinecap="round" />
    </>
  ),
  curl: (
    <>
      <path d="M8 14V6l1.33-1.5L10.67 6 12 4.5 13.33 6 14.67 4.5 16 6v8z" fill={INK} fillOpacity={0.72} />
      <rect x="9.4" y="7.4" width="3.6" height="1.1" rx="0.55" fill={ACCENT} />
      <rect x="9.4" y="9.6" width="5.2" height="0.9" rx="0.45" fill={ACCENT} fillOpacity={0.6} />
      <rect x="4.5" y="13.2" width="15" height="6.8" rx="2.6" fill={INK} />
      <rect x="7.4" y="13.2" width="9.2" height="1" rx="0.5" fill={ACCENT} fillOpacity={0.55} />
    </>
  ),
  ribbon: (
    <>
      <path d="M7.5 18.5V11a4.5 4.5 0 0 1 9 0v5.6" fill="none" stroke={INK} strokeWidth={3.2} strokeLinecap="round" />
      <rect x="5" y="17.9" width="5" height="1.8" rx="0.9" fill={INK} />
    </>
  ),
  roll: (
    <>
      <circle cx="9.5" cy="12.5" r="5" fill="none" stroke={INK} strokeWidth={2.2} />
      <circle cx="9.5" cy="12.5" r="1.3" fill={INK} />
      <path d="M9.5 17.5H18.5" fill="none" stroke={INK} strokeWidth={2.2} strokeLinecap="round" />
      <path d="M14.6 6.2a3.6 3.6 0 0 1 2.9 2.9" fill="none" stroke={INK} strokeWidth={1.8} strokeLinecap="round" />
      <path
        d="M15.3 3a7 7 0 0 1 5.6 5.6"
        fill="none"
        stroke={INK}
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeOpacity={0.6}
      />
    </>
  ),
};
