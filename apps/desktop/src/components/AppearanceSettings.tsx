import { useState } from "react";
import {
  ACCENTS,
  APP_ICONS,
  loadAccent,
  loadModePreference,
  setAccent,
  setModePreference,
  type AccentId,
  type ModePreference,
} from "../lib/theme";
import { IS_MAC } from "../lib/format";
import { api } from "../api";
import { useApp } from "../state/AppStore";
import { Segmented } from "./ui";

const MODES: { id: ModePreference; label: string }[] = [
  { id: "system", label: "Match system" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
];

/** Light or dark, the accent colour and the app icon. All remembered on this computer. */
export function AppearanceSettings() {
  const { appIcon, chooseAppIcon } = useApp();
  const [mode, setMode] = useState<ModePreference>(loadModePreference);
  const [accent, setCurrentAccent] = useState<AccentId>(loadAccent);

  return (
    <div className="settings-page">
      <div className="settings-head">
        <h2>Appearance</h2>
      </div>

      <Segmented
        label="Mode"
        value={mode}
        options={MODES}
        onChange={(m) => {
          setModePreference(m);
          setMode(m);
          api()
            .setWindowTheme(m === "system" ? null : m)
            .catch(() => undefined);
        }}
      />

      <fieldset className="choice-field">
        <legend className="field-label">Accent colour</legend>
        <div className="swatches">
          {ACCENTS.map((a) => (
            <label key={a.id} className={`swatch-option${a.id === accent ? " is-on" : ""}`}>
              <input
                type="radio"
                name="accent"
                value={a.id}
                checked={a.id === accent}
                onChange={() => {
                  setAccent(a.id);
                  setCurrentAccent(a.id);
                }}
              />
              <span className="accent-dot" data-swatch={a.id} aria-hidden="true" />
              <span>{a.label}</span>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="choice-field">
        <legend className="field-label">App icon</legend>
        <div className="icon-choices">
          {APP_ICONS.map((i) => (
            <label key={i.id} className={`icon-choice${i.id === appIcon ? " is-on" : ""}`}>
              <input
                type="radio"
                name="app-icon"
                value={i.id}
                checked={i.id === appIcon}
                onChange={() => chooseAppIcon(i.id)}
              />
              <img src={i.src} alt="" />
              <span>{i.label}</span>
            </label>
          ))}
        </div>
        <p className="hint">
          {IS_MAC
            ? "Shown in the Dock while NuPrint is open."
            : "Shown on the window and in the taskbar while NuPrint is open."}
        </p>
      </fieldset>
    </div>
  );
}
