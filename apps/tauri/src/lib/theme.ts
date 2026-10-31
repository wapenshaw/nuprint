/**
 * Accent themes and light/dark mode, applied to <html> as `data-accent` and `data-theme`
 * (styles.css keys palettes and dark tokens off them), and the chosen app icon; all
 * remembered in local storage.
 */

import cubeIcon from "../assets/app-icons/cube.svg";
import curlIcon from "../assets/app-icons/curl.svg";
import ribbonIcon from "../assets/app-icons/ribbon.svg";
import rollIcon from "../assets/app-icons/roll.svg";
import { readStored, writeStored } from "./storage";

// ---- accent

export type AccentId = "pink" | "indigo" | "coral" | "ocean" | "teal";

export const ACCENTS: readonly { id: AccentId; label: string }[] = [
  { id: "pink", label: "Pink" },
  { id: "indigo", label: "Indigo" },
  { id: "coral", label: "Coral" },
  { id: "ocean", label: "Ocean" },
  { id: "teal", label: "Teal" },
];

export const DEFAULT_ACCENT: AccentId = "pink";

const ACCENT_KEY = "nuprint.accent";

const isAccent = (value: unknown): value is AccentId => ACCENTS.some((a) => a.id === value);

/** The saved accent, or the default when nothing valid is stored. */
export function loadAccent(): AccentId {
  const saved = readStored(ACCENT_KEY);
  return isAccent(saved) ? saved : DEFAULT_ACCENT;
}

/** Apply an accent to the document and remember it. */
export function setAccent(accent: AccentId): void {
  document.documentElement.dataset.accent = accent;
  writeStored(ACCENT_KEY, accent);
}

/** Apply the saved accent; call before the first render so the wrong colour never flashes. */
export function applySavedAccent(): AccentId {
  const accent = loadAccent();
  document.documentElement.dataset.accent = accent;
  return accent;
}

// ---- light/dark mode: follows the OS unless the user picks Light or Dark in Settings.

export type ColorMode = "light" | "dark";

const MODE_KEY = "nuprint.mode";
const DARK_QUERY = "(prefers-color-scheme: dark)";

function systemMode(): ColorMode {
  return typeof window.matchMedia === "function" && window.matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

function modeOverride(): ColorMode | null {
  const saved = readStored(MODE_KEY);
  return saved === "light" || saved === "dark" ? saved : null;
}

function applyMode(mode: ColorMode): ColorMode {
  document.documentElement.dataset.theme = mode;
  return mode;
}

/** Apply the saved mode, else the system's; call before the first render. */
export function applySavedMode(): ColorMode {
  return applyMode(modeOverride() ?? systemMode());
}

/** What the user chose in Settings: follow the system, or always light or dark. */
export type ModePreference = "system" | ColorMode;

export function loadModePreference(): ModePreference {
  return modeOverride() ?? "system";
}

/** Apply and remember a preference; "system" clears the override and follows the OS. */
export function setModePreference(pref: ModePreference): ColorMode {
  writeStored(MODE_KEY, pref === "system" ? null : pref);
  return applyMode(pref === "system" ? systemMode() : pref);
}

/** Re-apply the system mode when the OS changes, while the user has no override. */
export function followSystemMode(onChange: (mode: ColorMode) => void): () => void {
  if (typeof window.matchMedia !== "function") return () => undefined;
  const query = window.matchMedia(DARK_QUERY);
  const listener = (e: MediaQueryListEvent) => {
    if (!modeOverride()) onChange(applyMode(e.matches ? "dark" : "light"));
  };
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}

// ---- app icon: the bundle ships the little printer (id `cube`, after its first design); the
// host shows another choice in the Dock (or on the window and taskbar) while the app runs.
// Add one here, as an SVG in assets/app-icons and as a 512 px PNG in
// src-tauri/icons/choices (see `set_app_icon`):
// `npx tauri icon <svg> -p 512 -o <dir>` renders one; without `-p` it makes the bundle set.

export type AppIconId = "cube" | "curl" | "ribbon" | "roll";

export const APP_ICONS: readonly { id: AppIconId; label: string; src: string }[] = [
  { id: "cube", label: "Little printer", src: cubeIcon },
  { id: "curl", label: "Curl", src: curlIcon },
  { id: "ribbon", label: "Ribbon", src: ribbonIcon },
  { id: "roll", label: "Air roll", src: rollIcon },
];

export const DEFAULT_APP_ICON: AppIconId = "cube";

const APP_ICON_KEY = "nuprint.icon";

const isAppIcon = (value: unknown): value is AppIconId => APP_ICONS.some((i) => i.id === value);

/** The saved app icon, or the default when nothing valid is stored. */
export function loadAppIcon(): AppIconId {
  const saved = readStored(APP_ICON_KEY);
  return isAppIcon(saved) ? saved : DEFAULT_APP_ICON;
}

/** Remember an app icon; the header passes it on to the host. */
export function saveAppIcon(icon: AppIconId): void {
  writeStored(APP_ICON_KEY, icon);
}
