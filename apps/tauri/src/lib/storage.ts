/** Choices remembered on this computer, in the web view's local storage. */

/** Storage can be unavailable (private mode, blocked site data); choices then last only this session. */
export function readStored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeStored(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // See readStored.
  }
}
