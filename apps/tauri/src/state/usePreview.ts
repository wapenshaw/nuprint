import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import type { AppError } from "../bindings/AppError";
import type { PreviewResult } from "../bindings/PreviewResult";
import type { SourceSpec } from "../bindings/SourceSpec";
import { isStalePreview, toAppError } from "../lib/errors";

export const PREVIEW_DEBOUNCE_MS = 250;

export interface PreviewState {
  /** A render for the current inputs is pending or in flight. */
  rendering: boolean;
  /** Last successful render (may be for older inputs while `rendering`). */
  result: PreviewResult | null;
  error: AppError | null;
  /** The displayed result was rendered for exactly the current inputs. */
  current: boolean;
  /** Nothing to render yet (e.g. no file opened). */
  idle: boolean;
}

interface Settled {
  key: string;
  result: PreviewResult | null;
  error: AppError | null;
}

// Monotonic across the whole session so a remounted view never reuses a generation. Seeded
// from the clock because Rust remembers the highest generation it has seen: a page reload
// that restarted at 0 would have every preview rejected as stale until the count caught up.
let generationCounter = Date.now();

/**
 * Debounced, generation-numbered preview rendering. Rust owns the actual rendering; this
 * hook only decides when to ask and which answer is still relevant.
 */
export function usePreview(
  profileId: string | null,
  source: SourceSpec | null,
  profileRevision = 0,
): PreviewState {
  const key = useMemo(
    () => (profileId && source ? JSON.stringify([profileId, profileRevision, source]) : null),
    [profileId, profileRevision, source],
  );
  const [settled, setSettled] = useState<Settled | null>(null);
  const [lastResult, setLastResult] = useState<PreviewResult | null>(null);
  const latestKey = useRef<string | null>(null);
  const latestGen = useRef(0);
  latestKey.current = key;

  useEffect(() => {
    if (!key || !profileId || !source) return;
    const timer = setTimeout(() => {
      const generation = ++generationCounter;
      latestGen.current = generation;
      api()
        .renderPreview({ generation, profile_id: profileId, source })
        .then((result) => {
          // A newer request started meanwhile: its answer is the one that matters.
          if (generation !== latestGen.current) return;
          setLastResult(result);
          setSettled({ key, result, error: null });
        })
        .catch((e: unknown) => {
          if (isStalePreview(e)) return;
          if (generation !== latestGen.current) return;
          setSettled({ key, result: null, error: toAppError(e) });
        });
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [key, profileId, source]);

  if (!key) {
    return { rendering: false, result: null, error: null, current: false, idle: true };
  }
  const isCurrent = settled?.key === key;
  const error = isCurrent ? settled.error : null;
  const result = isCurrent ? settled.result : error ? null : lastResult;
  return {
    rendering: !isCurrent,
    result,
    error,
    current: isCurrent && settled.result !== null && settled.error === null,
    idle: false,
  };
}
