import { within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { applySavedAccent, applySavedMode, DEFAULT_ACCENT, DEFAULT_APP_ICON, loadAppIcon } from "../lib/theme";
import { openSettings, renderApp } from "./helpers";

afterEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.accent;
  delete document.documentElement.dataset.theme;
});

describe("accent theme", () => {
  it("starts with the default accent and restores a saved one", () => {
    expect(applySavedAccent()).toBe(DEFAULT_ACCENT);
    localStorage.setItem("nuprint.accent", "teal");
    expect(applySavedAccent()).toBe("teal");
    expect(document.documentElement.dataset.accent).toBe("teal");
    localStorage.setItem("nuprint.accent", "green");
    expect(applySavedAccent()).toBe(DEFAULT_ACCENT);
  });

  it("lets the user pick an accent in Appearance and remembers it", async () => {
    const { user } = await renderApp();
    const settings = await openSettings(user, "Appearance");
    const accents = within(settings).getByRole("group", { name: "Accent colour" });
    expect(within(accents).getAllByRole("radio")).toHaveLength(5);
    expect(within(accents).getByRole("radio", { name: "Pink" })).toBeChecked();

    await user.click(within(accents).getByRole("radio", { name: "Coral" }));
    expect(document.documentElement.dataset.accent).toBe("coral");
    expect(localStorage.getItem("nuprint.accent")).toBe("coral");
  });

  it("follows the system mode until Light or Dark is picked", async () => {
    // jsdom has no matchMedia, so the system mode reads as light.
    expect(applySavedMode()).toBe("light");
    const { mock, user } = await renderApp();
    const settings = await openSettings(user, "Appearance");
    const mode = within(settings).getByRole("group", { name: "Mode" });
    expect(within(mode).getByRole("radio", { name: "Match system" })).toBeChecked();

    await user.click(within(mode).getByRole("radio", { name: "Dark" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem("nuprint.mode")).toBe("dark");
    expect(applySavedMode()).toBe("dark");
    expect(mock.calls.setWindowTheme.at(-1)).toBe("dark");

    await user.click(within(mode).getByRole("radio", { name: "Match system" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem("nuprint.mode")).toBeNull();
    expect(mock.calls.setWindowTheme.at(-1)).toBeNull();
  });

  it("lets the user pick an app icon, tells the host and remembers it", async () => {
    expect(loadAppIcon()).toBe(DEFAULT_APP_ICON);
    localStorage.setItem("nuprint.icon", "sparkles");
    expect(loadAppIcon()).toBe(DEFAULT_APP_ICON);

    const { mock, user } = await renderApp();
    // The saved (default) icon is applied once the app starts.
    expect(mock.calls.setAppIcon).toEqual(expect.arrayContaining(["cube"]));
    const settings = await openSettings(user, "Appearance");
    const icons = within(settings).getByRole("group", { name: "App icon" });
    expect(within(icons).getAllByRole("radio")).toHaveLength(4);

    await user.click(within(icons).getByRole("radio", { name: "Air roll" }));
    expect(localStorage.getItem("nuprint.icon")).toBe("roll");
    expect(mock.calls.setAppIcon.at(-1)).toBe("roll");
    expect(loadAppIcon()).toBe("roll");
  });
});
