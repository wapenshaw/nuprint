import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { closeSettings, openSettings, printButton, renderApp, waitForPrintEnabled, type User } from "./helpers";

async function openAdvanced(user: User) {
  await openSettings(user);
  await user.click(await screen.findByRole("button", { name: "Advanced settings for SC03h-TEST" }));
  const dialog = await screen.findByRole("dialog", { name: /Advanced settings/ });
  // The settings load after the dialog opens; a fast machine gets here before they arrive.
  await waitFor(() => expect(within(dialog).queryByText("Loading settings…")).not.toBeInTheDocument());
  return dialog;
}

async function setHeat(user: User, dialog: HTMLElement, label: string, value: string) {
  const field = within(dialog).getByLabelText(label);
  await user.clear(field);
  await user.type(field, value);
}

describe("advanced printer settings", () => {
  it("edits the heat, saves it with the printer and marks the printer as edited", async () => {
    const { user, mock } = await renderApp();
    const dialog = await openAdvanced(user);
    expect(within(dialog).getByText(/These values go straight to the printer/)).toBeInTheDocument();

    const save = within(dialog).getByRole("button", { name: "Save settings" });
    expect(save).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Reset to defaults" })).toBeDisabled();
    expect(within(dialog).getByLabelText("Heat: normal")).toHaveValue(5000);

    await setHeat(user, dialog, "Heat: normal", "4000");
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() => expect(mock.calls.savePrinterProfile).toHaveLength(1));
    const sent = JSON.parse(mock.calls.savePrinterProfile[0]!.json) as { driver: { energy: { normal: number } } };
    expect(sent.driver.energy.normal).toBe(4000);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /Advanced settings/ })).not.toBeInTheDocument());

    const saved = screen.getByRole("region", { name: "Saved printers" });
    await waitFor(() => expect(within(saved).getByText("Edited settings")).toBeInTheDocument());
    expect(within(saved).getByText("Experimental")).toBeInTheDocument();
    expect(document.body).toHaveTextContent(/Print a short test strip before relying on them/);

    // Reopening shows the saved values; Reset returns to the built-in profile.
    const again = await openAdvanced(user);
    expect(within(again).getByLabelText("Heat: normal")).toHaveValue(4000);
    await user.click(within(again).getByRole("button", { name: "Reset to defaults" }));
    await waitFor(() => expect(mock.calls.resetPrinterProfile).toHaveLength(1));
    await waitFor(() => expect(within(saved).queryByText("Edited settings")).not.toBeInTheDocument());
    expect(within(saved).getByText("Verified")).toBeInTheDocument();
  });

  it("flags a value that would be refused as it is typed, and blocks saving it", async () => {
    const { user, mock } = await renderApp();
    const dialog = await openAdvanced(user);
    const heat = within(dialog).getByLabelText("Heat: normal");
    const save = within(dialog).getByRole("button", { name: "Save settings" });

    await setHeat(user, dialog, "Heat: normal", "40000");
    expect(heat).toHaveAttribute("aria-invalid", "true");
    expect(within(dialog).getByText("Allowed: 1250–10000")).toBeInTheDocument();
    expect(save).toBeDisabled();

    await setHeat(user, dialog, "Heat: normal", "4000");
    expect(heat).toHaveAttribute("aria-invalid", "false");
    expect(within(dialog).queryByText(/Allowed:/)).not.toBeInTheDocument();
    expect(save).toBeEnabled();

    await setHeat(user, dialog, "Chunk size (bytes)", "5");
    expect(within(dialog).getByText("Allowed: 20–512")).toBeInTheDocument();
    expect(save).toBeDisabled();
    expect(mock.calls.savePrinterProfile).toHaveLength(0);
  });

  it("edits any setting through the JSON, and explains why a change was refused", async () => {
    const { user, mock } = await renderApp();
    const dialog = await openAdvanced(user);
    const json = within(dialog).getByLabelText("All editable settings (JSON)");

    await user.clear(json);
    await user.click(json);
    await user.paste('{ "match": { "name_prefixes": ["X"] } }');
    await user.click(within(dialog).getByRole("button", { name: "Save settings" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent('"match" cannot be changed');
    expect(screen.getByRole("dialog", { name: /Advanced settings/ })).toBeInTheDocument();

    await user.clear(json);
    await user.click(json);
    await user.paste("{ not json");
    expect(within(dialog).getByText(/not a valid JSON object/)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Save settings" })).toBeDisabled();
    expect(mock.calls.savePrinterProfile).toHaveLength(1);
  });

  it("exports the saved profile through the native dialog", async () => {
    const { user, mock } = await renderApp();
    const dialog = await openAdvanced(user);
    await user.click(within(dialog).getByRole("button", { name: "Export profile…" }));
    await waitFor(() => expect(mock.calls.exportPrinterProfile).toEqual(["printer-seed"]));
    await waitFor(() => expect(document.body).toHaveTextContent(/Saved the printer profile file/));
  });

  it("prints with the edited profile: a new preview is requested for the new revision", async () => {
    const { user, mock } = await renderApp();
    await waitForPrintEnabled();
    const before = mock.calls.renderPreview.length;
    const dialog = await openAdvanced(user);
    await setHeat(user, dialog, "Heat: normal", "4100");
    await user.click(within(dialog).getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /Advanced settings/ })).not.toBeInTheDocument());

    await closeSettings(user);
    await waitFor(() => expect(mock.calls.renderPreview.length).toBeGreaterThan(before));
    expect(mock.calls.renderPreview.at(-1)!.profile_id).toMatch(/^custom:/);
    await waitForPrintEnabled();
    await user.click(printButton());
    await waitFor(() => expect(mock.calls.enqueue).toHaveLength(1));
  });
});
