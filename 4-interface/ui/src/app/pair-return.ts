// Where to go after pairing. A visitor who opens `#/devices` and gets a 401 is sent to `/pair`; after the
// code is accepted they should land on `/devices`, not on the Agent. The route is kept in sessionStorage
// (this tab only) and consumed once.

const KEY = "111.ui.return";

export interface ReturnStore {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
}

function defaultStore(): ReturnStore | null {
  try {
    return typeof sessionStorage !== "undefined" ? sessionStorage : null;
  } catch {
    return null;
  }
}

// A plain in-app route path: no scheme, no `//`, no query or fragment, and never the pair screen itself.
const SAFE = /^\/[A-Za-z0-9._~%!$&'()*+,;=:@/-]*$/;

/** True for a route worth returning to. */
export function isReturnable(path: string | null | undefined): path is string {
  return typeof path === "string" && path !== "/pair" && path !== "/" && SAFE.test(path) && !path.startsWith("//") && path.length <= 256;
}

/** Remember the route the visitor was on when the node asked them to pair. Ignores anything not returnable. */
export function rememberReturn(path: string, store: ReturnStore | null = defaultStore()): void {
  if (!isReturnable(path)) return;
  try {
    store?.setItem(KEY, path);
  } catch {
    /* storage unavailable: pairing lands on the Agent */
  }
}

/** The remembered route (consumed), or `/` when there is none or it is not safe. */
export function takeReturn(store: ReturnStore | null = defaultStore()): string {
  try {
    const path = store?.getItem(KEY);
    store?.removeItem(KEY);
    return isReturnable(path) ? path : "/";
  } catch {
    return "/";
  }
}
