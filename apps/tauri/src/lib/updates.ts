/**
 * Whether NuPrint checks for a newer release when it starts. Off until the user turns it on:
 * NuPrint contacts nothing but the printer unless asked to.
 */

import { readStored, writeStored } from "./storage";

const CHECK_AT_START_KEY = "nuprint.updates.checkAtStart";

export function loadCheckAtStart(): boolean {
  return readStored(CHECK_AT_START_KEY) === "on";
}

export function saveCheckAtStart(on: boolean): void {
  writeStored(CHECK_AT_START_KEY, on ? "on" : null);
}
