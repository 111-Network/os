// Pair code helpers (spec section 11): 8 characters shown as xxxx-xxxx. The daemon ignores dashes,
// spaces and case, so the field only has to keep the user's typing tidy.

export const PAIR_CODE_LENGTH = 8;

/** Uppercase, drop everything that is not a letter or digit, cap at 8, and insert the dash after 4. */
export function formatPairCode(raw: string): string {
  const chars = raw.replace(/[^A-Za-z0-9]/g, "").toUpperCase().slice(0, PAIR_CODE_LENGTH);
  return chars.length > 4 ? `${chars.slice(0, 4)}-${chars.slice(4)}` : chars;
}

export function isCompletePairCode(formatted: string): boolean {
  return formatted.replace("-", "").length === PAIR_CODE_LENGTH;
}

export const DEFAULT_RETRY_SECONDS = 60;

/**
 * How long a 429 asks the visitor to wait. Reads the problem `detail` when it names a time
 * ("wait 45 seconds", "try again in 2 minutes", "a minute"); otherwise the daemon's window of 60 s.
 */
export function retrySeconds(detail: string | undefined): number {
  if (!detail) return DEFAULT_RETRY_SECONDS;
  const n = /(\d+)\s*(seconds?|secs?|s)\b/i.exec(detail);
  if (n?.[1]) return Math.max(1, Number(n[1]));
  const m = /(\d+)\s*(minutes?|mins?)\b/i.exec(detail);
  if (m?.[1]) return Math.max(1, Number(m[1]) * 60);
  return DEFAULT_RETRY_SECONDS;
}
