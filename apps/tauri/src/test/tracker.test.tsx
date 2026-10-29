import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { printButton, renderApp, waitForPrintEnabled } from "./helpers";

const latest = () => screen.getByLabelText("Latest print job");

describe("status bar job", () => {
  it("shows a printed job in the status bar and can cancel it there", async () => {
    const { user, mock } = await renderApp();
    expect(screen.queryByLabelText("Latest print job")).not.toBeInTheDocument();
    await waitForPrintEnabled();
    await user.click(printButton());

    expect(await screen.findByLabelText("Latest print job")).toHaveTextContent("Waiting");
    expect(document.body).toHaveTextContent(/Added "Hello from NuPrint!"/);

    await user.click(screen.getByRole("button", { name: "Cancel Hello from NuPrint!" }));
    await waitFor(() => expect(mock.calls.cancel).toHaveLength(1));
    await waitFor(() => expect(within(latest()).getByText("Cancelled")).toBeInTheDocument());
  });

  it("follows the job to completion, then dismisses it", async () => {
    const { user } = await renderApp({ autoAdvance: true, stepMs: 40 });
    await waitForPrintEnabled();
    await user.click(printButton());

    await waitFor(() => expect(within(latest()).getByText("Done · printer ready")).toBeInTheDocument(), {
      timeout: 4000,
    });
    await user.click(screen.getByRole("button", { name: "Dismiss message" }));
    expect(screen.queryByLabelText("Latest print job")).not.toBeInTheDocument();
  });

  it("does not show queue history that was not just printed", async () => {
    await renderApp({}, (m) => {
      m.seedJob({ state: "sending", title: "First", bytes_sent: 100, bytes_total: 1000 });
    });
    expect(screen.queryByLabelText("Latest print job")).not.toBeInTheDocument();
  });

  it("opens settings with Ctrl+, and closes them with Escape", async () => {
    const { user } = await renderApp();
    await user.keyboard("{Control>},{/Control}");
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Printers" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument();
  });
});
