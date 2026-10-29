import { screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SAMPLE_UPDATE as UPDATE } from "../api/mock";
import { openSettings, renderApp } from "./helpers";

afterEach(() => localStorage.clear());

describe("updates", () => {
  it("checks nothing until asked, then says when this is the newest version", async () => {
    const { mock, user } = await renderApp();
    expect(mock.calls.checkForUpdate).toBe(0);

    const settings = await openSettings(user, "Updates");
    expect(within(settings).getByRole("checkbox", { name: "Check for updates when NuPrint starts" })).not.toBeChecked();
    await user.click(within(settings).getByRole("button", { name: "Check for updates" }));
    expect(await within(settings).findByText("This is the newest version.")).toBeInTheDocument();
    expect(mock.calls.checkForUpdate).toBe(1);
  });

  it("checks at start when turned on and offers the update with what changed", async () => {
    localStorage.setItem("nuprint.updates.checkAtStart", "on");
    const { mock, user } = await renderApp({ update: UPDATE });

    expect(await screen.findByText("NuPrint 1.2.0 is available.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "See what’s new" }));
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    expect(within(settings).getByRole("heading", { name: "NuPrint 1.2.0 is available" })).toBeInTheDocument();
    expect(within(settings).getByLabelText("What’s new")).toHaveTextContent("Check for updates");

    await user.click(within(settings).getByRole("button", { name: "Install and restart" }));
    expect(mock.calls.installUpdate).toBe(1);
  });

  it("remembers the choice to check at start", async () => {
    const { user } = await renderApp();
    const settings = await openSettings(user, "Updates");
    await user.click(within(settings).getByRole("checkbox", { name: "Check for updates when NuPrint starts" }));
    expect(localStorage.getItem("nuprint.updates.checkAtStart")).toBe("on");
  });

  it("keeps one install going when the user leaves the page and comes back", async () => {
    const { mock, user } = await renderApp({ update: UPDATE });
    const settings = await openSettings(user, "Updates");
    await user.click(within(settings).getByRole("button", { name: "Check for updates" }));
    await user.click(await within(settings).findByRole("button", { name: "Install and restart" }));

    await user.click(within(settings).getByRole("button", { name: "Appearance" }));
    await user.click(within(settings).getByRole("button", { name: "Updates" }));
    expect(await within(settings).findByRole("button", { name: "Installing…" })).toBeDisabled();
    expect(mock.calls.installUpdate).toBe(1);
  });

  it("shows a failed install and lets the user try again", async () => {
    const { mock, user } = await renderApp({ update: UPDATE });
    const settings = await openSettings(user, "Updates");
    await user.click(within(settings).getByRole("button", { name: "Check for updates" }));
    vi.spyOn(mock, "installUpdate").mockRejectedValueOnce({
      code: "internal",
      message: "NuPrint could not install the update.",
    });
    await user.click(await within(settings).findByRole("button", { name: "Install and restart" }));
    expect(await screen.findByText("NuPrint could not install the update.")).toBeInTheDocument();

    await user.click(within(settings).getByRole("button", { name: "Install and restart" }));
    expect(mock.calls.installUpdate).toBe(1);
  });

  it("does not install while a job is printing", async () => {
    const { mock, user } = await renderApp({ update: UPDATE });
    mock.seedJob({ state: "sending" });
    const settings = await openSettings(user, "Updates");
    await user.click(within(settings).getByRole("button", { name: "Check for updates" }));
    expect(await within(settings).findByRole("button", { name: "Install and restart" })).toBeDisabled();
    expect(within(settings).getByText("A job is printing. Install the update when it has finished.")).toBeInTheDocument();
  });
});
