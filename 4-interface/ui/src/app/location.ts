// Hash-route location for wouter. `useHashLocation` returns everything after `#`, including a
// `?query` and a second `#fragment` (for example `#/overview#needs-you` or `#/pair?code=...`).
// Routes match on the path only, so the extras are stripped here. Views that need the fragment
// read it with `routeFragment()`.
import { useHashLocation } from "wouter/use-hash-location";

/** `/overview#needs-you` -> `/overview`; `/pair?code=x` -> `/pair`; `/devices/` -> `/devices`; empty -> `/`. */
export function cleanPath(raw: string): string {
  const cut = raw.search(/[?#]/);
  const path = cut < 0 ? raw : raw.slice(0, cut);
  const trimmed = path.replace(/\/+$/, "");
  if (trimmed === "") return "/";
  return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

/** The fragment inside the hash route: `#/overview#needs-you` -> `needs-you`; none -> null. */
export function routeFragment(hash: string): string | null {
  const body = hash.replace(/^#/, "");
  const i = body.indexOf("#");
  if (i < 0) return null;
  const frag = body.slice(i + 1);
  return frag === "" ? null : frag;
}

/** Link target for a route path. */
export const hrefTo = (path: string): string => `#${path}`;

export function useAppLocation(): ReturnType<typeof useHashLocation> {
  const [raw, navigate] = useHashLocation();
  return [cleanPath(raw), navigate];
}
// Lets wouter's <Link> render hash hrefs (the stock hook carries this only at runtime).
useAppLocation.hrefs = hrefTo;
