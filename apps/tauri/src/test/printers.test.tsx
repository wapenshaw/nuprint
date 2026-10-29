import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { describe, expect, it } from "vitest";
import App from "../App";
import { setBackend } from "../api";
import { createMockBackend } from "../api/mock";
import { openSettings, renderApp, type User } from "./helpers";

async function scanFor(user: User, name: string) {
  await openSettings(user);
  await user.click(await screen.findByRole("button", { name: "Find printers" }));
  await screen.findByText(name, undefined, { timeout: 3000 });
}

describe("printers", () => {
  it("requires an explicit acknowledgement before saving an experimental printer", async () => {
    const { user, mock } = await renderApp({ seedPrinters: false });
    await scanFor(user, "DemoWide-80");

    await user.click(screen.getByRole("button", { name: "Save DemoWide-80" }));
    const dialog = await screen.findByRole("dialog", { name: /Save DemoWide-80/ });
    expect(within(dialog).getByText(/Experimental: this model has not been tested/)).toBeInTheDocument();

    const save = within(dialog).getByRole("button", { name: "Save printer" });
    expect(save).toBeDisabled();
    expect(mock.calls.savePrinter).toHaveLength(0);

    await user.click(within(dialog).getByRole("checkbox", { name: /I understand this printer is experimental/ }));
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() => expect(mock.calls.savePrinter).toHaveLength(1));
    expect(mock.calls.savePrinter[0]).toMatchObject({ profile_id: "demo-wide", acknowledge_experimental: true });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /Save DemoWide-80/ })).not.toBeInTheDocument());
    // Saved printers appear in the list with their support level as text.
    const saved = screen.getByRole("region", { name: "Saved printers" });
    expect(within(saved).getByText("DemoWide-80")).toBeInTheDocument();
    expect(within(saved).getByText("Experimental")).toBeInTheDocument();
  });

  it("saves a verified printer without an acknowledgement, and Escape closes the dialog", async () => {
    const { user, mock } = await renderApp({ seedPrinters: false });
    await scanFor(user, "SC03h-TEST");

    await user.click(screen.getByRole("button", { name: "Save SC03h-TEST" }));
    await screen.findByRole("dialog", { name: /Save SC03h-TEST/ });
    // Escape closes only the dialog on top; settings stay open under it.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: /Save SC03h-TEST/ })).not.toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Save SC03h-TEST" }));
    const dialog = await screen.findByRole("dialog", { name: /Save SC03h-TEST/ });
    expect(within(dialog).queryByRole("checkbox")).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Save printer" }));
    await waitFor(() => expect(mock.calls.savePrinter[0]).toMatchObject({ acknowledge_experimental: false }));
  });

  it("inspects once in strict mode, and closing while connecting cancels the inspection", async () => {
    // The app renders in StrictMode, which runs effects twice in development.
    const mock = createMockBackend({ latencyMs: 5, autoAdvance: false, scanMs: 300, seedPrinters: false });
    setBackend(mock);
    const user = userEvent.setup();
    render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
    await waitFor(() => expect(screen.queryByText("Loading…")).not.toBeInTheDocument());
    await scanFor(user, "SC03h-TEST");

    await user.click(screen.getByRole("button", { name: "Inspect SC03h-TEST" }));
    await screen.findByText(/Connecting briefly/);
    await user.keyboard("{Escape}");
    expect(mock.calls.cancelInspect).toBe(1);

    await user.click(screen.getByRole("button", { name: "Inspect SC03h-TEST" }));
    const dialog = await screen.findByRole("dialog", { name: /Inspect SC03h-TEST/ });
    await within(dialog).findByText("Compatible printer profiles");
    await user.keyboard("{Escape}");
    expect(mock.calls.cancelInspect).toBe(1);
    expect(mock.calls.inspect).toHaveLength(2);
  });

  it("on macOS, explains the permission prompt and offers the settings when access is not given", async () => {
    const { user, mock } = await renderApp({ adapter: "permission_not_asked", os: "macos", scanMs: 300 });
    await openSettings(user);
    expect(screen.getByTestId("adapter-status")).toHaveTextContent(/macOS will ask whether NuPrint may use Bluetooth/);
    expect(screen.queryByRole("button", { name: "Open Bluetooth settings" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Find printers" }));
    expect(await screen.findByText(/choose Allow\. The scan continues once you answer/)).toBeInTheDocument();
    expect(await screen.findByText("NuPrint does not have Bluetooth access yet.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open Bluetooth settings" }));
    expect(mock.calls.openBluetoothSettings).toBe(1);
  });

  it("offers the macOS settings only on macOS", async () => {
    const { user } = await renderApp({ adapter: "permission_denied" });
    await openSettings(user);
    expect(screen.getByTestId("adapter-status")).toHaveTextContent(/not allowed to use Bluetooth/);
    expect(screen.queryByRole("button", { name: "Open Bluetooth settings" })).not.toBeInTheDocument();
  });

  it("shows distinct adapter messages", async () => {
    const { user } = await renderApp({ adapter: "powered_off" });
    await openSettings(user);
    expect(screen.getByTestId("adapter-status")).toHaveTextContent(/Bluetooth is off/);
    expect(screen.getByTestId("adapter-status")).not.toHaveTextContent(/root|sudo/i);
  });

  it("checks a saved printer connection without printing", async () => {
    const { user, mock } = await renderApp();
    await openSettings(user);
    expect(screen.getByRole("img", { name: "SC03h-TEST: Not checked yet" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Check connection for SC03h-TEST" }));
    expect(await screen.findByText(/Ready · paper present/)).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "SC03h-TEST: Ready" })).toBeInTheDocument();
    expect(mock.calls.enqueue).toHaveLength(0);
  });
});

describe("printer menu", () => {
  it("shows the printer in the toolbar, checks its connection and opens printer settings", async () => {
    const { user, mock } = await renderApp();
    const button = screen.getByRole("button", { name: /SC03h-TEST/ });
    expect(button).toHaveTextContent("Not checked yet");

    await user.click(button);
    const printTo = screen.getByRole("group", { name: "Print to" });
    expect(within(printTo).getByRole("radio", { name: /SC03h-TEST/ })).toBeChecked();
    await user.click(screen.getByRole("button", { name: /Check connection/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: /SC03h-TEST/ })).toHaveTextContent("Ready"));
    expect(mock.calls.enqueue).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: "Manage printers…" }));
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Print to" })).not.toBeInTheDocument();
  });
});
