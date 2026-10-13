// Routes the browser to /pair when the node says it is not paired. The store already turns any 401
// on a snapshot call (and a fatal /events close, which probes status) into `meta.needsPair`; this
// component only reacts to that flag. Views that catch an ApiError from an action can call
// `redirectIfUnauthenticated(err)`.
import { useEffect } from "react";
import { navigate as hashNavigate } from "wouter/use-hash-location";
import { useLocation } from "wouter";
import { isApiError } from "../lib/api";
import { useApiMeta } from "../lib/api/hooks";

export const PAIR_PATH = "/pair";

/** True (and navigates to /pair) when `err` is a 401 `unauthenticated` ApiError. */
export function redirectIfUnauthenticated(err: unknown): boolean {
  if (!isApiError(err) || !err.unauthenticated) return false;
  hashNavigate(PAIR_PATH, { replace: true });
  return true;
}

export function AuthGate() {
  const { needsPair } = useApiMeta();
  const [path, navigate] = useLocation();
  useEffect(() => {
    // Never redirect while already on /pair: a wrong code must not loop back to the same screen.
    if (needsPair && path !== PAIR_PATH) navigate(PAIR_PATH, { replace: true });
  }, [needsPair, path, navigate]);
  return null;
}
