import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "../App";
import { setBackend } from "../api";
import { createMockBackend, type MockBackend, type MockOptions } from "../api/mock";

export type User = ReturnType<typeof userEvent.setup>;

export async function renderApp(options: MockOptions = {}, prepare?: (mock: MockBackend) => void) {
  const mock = createMockBackend({ latencyMs: 5, autoAdvance: false, scanMs: 300, ...options });
  prepare?.(mock);
  setBackend(mock);
  const user = userEvent.setup();
  render(<App />);
  // Wait for the initial snapshot to load.
  await waitFor(() => expect(screen.queryByText("Loading…")).not.toBeInTheDocument());
  return { mock, user };
}

/** The Print button in the print panel (exact name, so e.g. "Print X again" buttons do not match). */
export const printButton = () => screen.getByRole("button", { name: /^Print$|^Adding to queue…$/ });

export async function waitForPrintEnabled() {
  await waitFor(() => expect(printButton()).toBeEnabled(), { timeout: 4000 });
}

export async function goTo(user: User, name: RegExp | string) {
  await user.click(screen.getByRole("button", { name }));
}

/** Open the settings window from the toolbar, at a section (it opens at Printers). */
export async function openSettings(user: User, section = "Printers") {
  await user.click(screen.getByRole("button", { name: "Settings" }));
  const dialog = await screen.findByRole("dialog", { name: "Settings" });
  if (section !== "Printers") await user.click(within(dialog).getByRole("button", { name: section }));
  return dialog;
}

export async function closeSettings(user: User) {
  await user.click(screen.getByRole("button", { name: "Close settings" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument());
}
