import type { AppError } from "../bindings/AppError";

export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { code?: unknown }).code === "string" &&
    typeof (e as { message?: unknown }).message === "string"
  );
}

/** Normalise any rejection into an AppError (Tauri rejects with the serialized AppError). */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  if (e instanceof Error) return { code: "internal", message: e.message };
  if (typeof e === "string") return { code: "internal", message: e };
  return { code: "internal", message: "Something went wrong." };
}

export function errorMessage(e: unknown): string {
  return toAppError(e).message;
}

/** Stale previews are expected when a newer render superseded this one; ignore silently. */
export function isStalePreview(e: unknown): boolean {
  return isAppError(e) && e.code === "stale_preview";
}

export function needsPdfPassword(e: unknown): boolean {
  return isAppError(e) && e.code === "pdf_password_required";
}
