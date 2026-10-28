import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { goTo, renderApp, waitForPrintEnabled, printButton } from "./helpers";

describe("queue", () => {
  it("shows friendly state labels, byte progress and a Cancel button for the active job", async () => {
    const { user, mock } = await renderApp({}, (m) => {
      m.seedJob({
        state: "sending",
        title: "Receipt",
        bytes_sent: 4000,
        bytes_total: 10000,
        state_text: "Sending data to the printer…",
      });
      m.seedJob({ state: "queued", title: "Second note" });
      m.seedJob({ state: "completed", title: "Old note", state_text: "Transfer complete · printer reports ready" });
      m.seedJob({
        state: "uncertain",
        title: "Lost link",
        partial_output_possible: true,
        state_text: "Connection lost after sending",
      });
      m.seedJob({
        state: "failed",
        title: "Broken",
        content_retained: false,
        error: { code: "paper_out", message: "The printer is out of paper." },
      });
    });
    await goTo(user, /^Queue/);

    expect(screen.getByText("Sending")).toBeInTheDocument();
    expect(screen.getByText("Waiting")).toBeInTheDocument();
    expect(screen.getByText("Done · printer ready")).toBeInTheDocument();
    expect(screen.getByText("Uncertain — check paper")).toBeInTheDocument();
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Transfer complete · printer reports ready")).toBeInTheDocument();
    expect(screen.getByText(/Some paper may have printed/)).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("The printer is out of paper.");

    // Transfer progress is labelled as data transfer, not page progress.
    const bar = screen.getByRole("progressbar", { name: /Data sent to printer for Receipt/ });
    expect(bar).toHaveAttribute("value", "4000");
    expect(screen.getByText(/This is not page progress/)).toBeInTheDocument();

    // Actions by state.
    expect(screen.getByRole("button", { name: "Cancel Receipt" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Cancel Second note" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Cancel Old note" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Print Old note again as a new job/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Print Broken again/ })).not.toBeInTheDocument(); // content not retained
    expect(screen.getByRole("button", { name: /Remove Old note/ })).toBeInTheDocument();

    // The queue badge counts open jobs only (sending + queued).
    expect(screen.getByLabelText("2 jobs in progress")).toBeInTheDocument();

    // Newest first.
    const items = within(screen.getByRole("list", { name: /Print jobs/ })).getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Broken");

    await user.click(screen.getByRole("button", { name: "Cancel Receipt" }));
    await waitFor(() => expect(mock.calls.cancel).toHaveLength(1));
    await waitFor(() => expect(screen.getByText("Cancelled")).toBeInTheDocument());
  });

  it("warns about jobs held after a restart and toggles hold/resume", async () => {
    const { user } = await renderApp({}, (m) => {
      m.seedJob({ state: "held", title: "Held note" });
    });
    await goTo(user, /^Queue/);
    expect(screen.getByText(/Jobs were held after restart/)).toBeInTheDocument();
    expect(screen.getByText("Held")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Hold queue" }));
    expect(await screen.findByRole("button", { name: "Resume queue" })).toBeInTheDocument();
    expect(screen.getByText("Queue held:")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Resume queue" }));
    await waitFor(() => expect(screen.getByText("Waiting")).toBeInTheDocument());
    expect(screen.queryByText(/Jobs were held after restart/)).not.toBeInTheDocument();
  });

  it("draws each job as its receipt while the content is kept", async () => {
    const { user, mock } = await renderApp();
    await waitForPrintEnabled();
    await user.click(printButton());
    await goTo(user, /^Queue/);
    const row = within(screen.getByRole("list", { name: /Print jobs/ })).getAllByRole("listitem")[0]!;
    const img = row.querySelector(".receipt-paper img");
    expect(img).not.toBeNull();
    const jobId = (await mock.listJobs()).jobs[0]!.id;
    expect(mock.jobPreviewUrl(jobId)).not.toBe("");
    expect(img!.getAttribute("src")).toBe(mock.jobPreviewUrl(jobId));
  });

  it("follows a printed job through to completion via events", async () => {
    const { user } = await renderApp({ autoAdvance: true, stepMs: 40 });
    await waitForPrintEnabled();
    await user.click(printButton());
    await goTo(user, /^Queue/);
    await waitFor(() => expect(within(screen.getByRole("list", { name: /Print jobs/ })).getByText("Done · printer ready")).toBeInTheDocument(), {
      timeout: 4000,
    });
  });
});
