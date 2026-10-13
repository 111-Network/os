// Canonical URL form is `/?p=v#/route`: parameters live in the document query. A parameter may
// also arrive inside the hash (`#/route?p=v`, e.g. the QR code's `#/pair?code=...`). When both
// carry the same name the document wins. Pure functions with injected side effects.

/** The query part of a hash like `#/route?a=b`, as URLSearchParams. */
function hashQuery(hash: string): URLSearchParams {
  const q = hash.indexOf("?");
  return new URLSearchParams(q < 0 ? "" : hash.slice(q + 1));
}

function hashPath(hash: string): string {
  const q = hash.indexOf("?");
  return q < 0 ? hash : hash.slice(0, q);
}

/** Merge both sources; document wins. */
export function readParams(search: string, hash: string): URLSearchParams {
  const merged = hashQuery(hash);
  for (const [k, v] of new URLSearchParams(search)) merged.set(k, v);
  return merged;
}

export interface ConsumeDeps {
  /** `location.search`, with or without the leading "?". */
  search: string;
  /** `location.hash`, with or without the leading "#". Default "". */
  hash?: string;
  /** Called only when something was stripped. `newSearch` starts with "?" or is ""; `newHash` starts with "#" or is "". */
  replace: (newSearch: string, newHash: string) => void;
}

/**
 * Read `name` (document first, then the hash query) and strip it from both places, so a one-shot
 * parameter such as `?mode=advanced` or `?code=...` is applied once and does not linger in the
 * address bar or get bookmarked. Returns null when absent.
 */
export function consumeOnce(name: string, deps: ConsumeDeps): string | null {
  const search = new URLSearchParams(deps.search);
  const hash = deps.hash ?? "";
  const hq = hashQuery(hash);
  const value = search.get(name) ?? hq.get(name);
  if (value === null) return null;
  search.delete(name);
  hq.delete(name);
  const s = search.toString();
  const h = hq.toString();
  const path = hashPath(hash).replace(/^#/, "");
  deps.replace(s ? `?${s}` : "", h ? `#${path}?${h}` : path ? `#${path}` : "");
  return value;
}

/** Browser convenience: consumeOnce against window.location, rewriting the URL with replaceState. */
export function consumeParam(name: string): string | null {
  return consumeOnce(name, {
    search: window.location.search,
    hash: window.location.hash,
    replace: (newSearch, newHash) => {
      window.history.replaceState(window.history.state, "", `${window.location.pathname}${newSearch}${newHash}`);
    },
  });
}
