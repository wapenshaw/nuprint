import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { printButton, renderApp, waitForPrintEnabled } from "./helpers";

describe("workspace printing", () => {
  it("disables Print while the preview renders and after every edit, enabling it once the new preview is shown", async () => {
    const { user } = await renderApp();
    expect(printButton()).toBeDisabled();
    await waitForPrintEnabled();

    await user.type(screen.getByRole("textbox", { name: "Text" }), " more");
    // The edit makes the displayed snapshot stale immediately (before the debounce even fires).
    expect(printButton()).toBeDisabled();
    expect(screen.getAllByText("Rendering…").length).toBeGreaterThan(0);

    await waitForPrintEnabled();
    expect(screen.getByText(/dots · ≈/)).toBeInTheDocument();
  });

  it("enqueues exactly once when Print is double clicked", async () => {
    const { user, mock } = await renderApp();
    await waitForPrintEnabled();

    await user.dblClick(printButton());
    await waitFor(() => expect(document.body).toHaveTextContent(/Added .* to the print queue/));
    expect(mock.calls.enqueue).toHaveLength(1);
    const req = mock.calls.enqueue[0]!;
    expect(req.title).toBe("Hello from NuPrint!");
    expect(req.copies).toBe(1);
    expect(req.feed_mm).toBe(12);
    expect(req.idempotency_key).toMatch(/[0-9a-f-]{36}/);
  });

  it("also guards against two clicks fired in the same tick", async () => {
    const { mock } = await renderApp();
    await waitForPrintEnabled();
    const btn = printButton();
    fireEvent.click(btn);
    fireEvent.click(btn);
    await waitFor(() => expect(mock.calls.enqueue).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 50));
    expect(mock.calls.enqueue).toHaveLength(1);
  });

  it("prints with Ctrl+P and saves the PNG with Ctrl+S", async () => {
    const { user, mock } = await renderApp();
    await waitForPrintEnabled();
    await user.keyboard("{Control>}p{/Control}");
    await waitFor(() => expect(mock.calls.enqueue).toHaveLength(1));
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(mock.calls.savePng).toHaveLength(1));
  });

  it("clamps the extra feed to the profile bounds and explains zero feed", async () => {
    const { user, mock } = await renderApp();
    const feed = (await screen.findByLabelText("Extra feed (mm)")) as HTMLInputElement;
    expect(feed.value).toBe("12");

    await user.clear(feed);
    await user.type(feed, "99");
    expect(feed.value).toBe("12"); // profile max
    await user.clear(feed);
    await user.type(feed, "-5");
    expect(feed.value).toBe("0"); // profile min

    await user.hover(screen.getByRole("button", { name: "About extra feed" }));
    const hint = await screen.findByRole("tooltip");
    expect(hint).toHaveTextContent("0 = no extra feed");
    expect(hint).toHaveTextContent("Less feed may leave the last line inside the printer");
    expect(hint).toHaveTextContent("have not been measured on real paper yet"); // calibrated_reduced=false

    // The clamped value (not the raw keystrokes) is what gets persisted for the printer.
    await waitFor(() => expect(mock.calls.updateSettings.at(-1)?.feed_mm).toBe(0));
    expect(mock.calls.updateSettings.every((c) => c.feed_mm >= 0 && c.feed_mm <= 12)).toBe(true);
  });

  it("bounds copies by the profile and includes feed once in the paper estimate", async () => {
    const { user } = await renderApp();
    await waitForPrintEnabled();
    const copies = screen.getByLabelText("Copies") as HTMLInputElement;
    await user.clear(copies);
    await user.type(copies, "500");
    expect(copies.value).toBe("20");
    await user.clear(copies);
    await user.type(copies, "3");
    const est = screen.getByTestId("paper-estimate");
    expect(est).toHaveTextContent("3 ×");
    expect(est).toHaveTextContent("+ 12 mm feed");

    // The preview strip shows every copy; only the first is announced.
    await waitFor(() => expect(screen.getByTestId("paper").querySelectorAll("img")).toHaveLength(3));
    expect(screen.getAllByRole("img", { name: /^Print preview/ })).toHaveLength(1);
    expect(screen.getByText(/× 3 copies, then 12 mm extra feed/)).toBeInTheDocument();
  });

  it("ignores stale_preview rejections silently", async () => {
    const { user, mock } = await renderApp({}, (m) => {
      m.previewErrors.push({ code: "stale_preview", message: "A newer preview was requested." });
    });
    await waitFor(() => expect(mock.calls.renderPreview).toHaveLength(1), { timeout: 3000 });
    await new Promise((r) => setTimeout(r, 80));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(printButton()).toBeDisabled();

    // A newer generation then renders normally.
    await user.type(screen.getByRole("textbox", { name: "Text" }), "!");
    await waitForPrintEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(mock.calls.renderPreview[1]!.generation).toBeGreaterThan(mock.calls.renderPreview[0]!.generation);
  });

  it("does not show an out-of-order older preview", async () => {
    const { user, mock } = await renderApp({ previewLatencyMs: (n) => (n === 1 ? 700 : 20) });
    await waitFor(() => expect(mock.calls.renderPreview).toHaveLength(1), { timeout: 3000 });
    await user.type(screen.getByRole("textbox", { name: "Text" }), "x");
    await waitForPrintEnabled();
    await new Promise((r) => setTimeout(r, 900)); // let the slow first render finish and be rejected
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(printButton()).toBeEnabled();
  });

  it("shows preview errors inline and keeps Print disabled", async () => {
    const { mock } = await renderApp({}, (m) => {
      m.previewErrors.push({ code: "render_invalid", message: "This text cannot be drawn." });
    });
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("This text cannot be drawn."), {
      timeout: 3000,
    });
    expect(printButton()).toBeDisabled();
    expect(mock.calls.enqueue).toHaveLength(0);
  });

  it("explains how to add a printer when none is saved", async () => {
    const { user } = await renderApp({ seedPrinters: false });
    const panel = screen.getByRole("region", { name: "Print" });
    expect(within(panel).getByText(/No printer yet/)).toBeInTheDocument();
    expect(printButton()).toBeDisabled();
    await user.click(within(panel).getByRole("button", { name: "Find printers…" }));
    expect(await screen.findByRole("heading", { name: "Printers" })).toBeInTheDocument();
    expect(await screen.findByText(/Scanning for about 8 seconds/)).toBeInTheDocument();
  });
});
