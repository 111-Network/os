// Pair screen (spec sections 6.9 and 11). Shown when the node answers 401 to a LAN visitor; loopback
// visitors never see it. A wrong code stays on this screen (the auth gate never redirects from /pair).
import { useEffect, useState, type FormEvent } from "react";
import { navigate } from "wouter/use-hash-location";
import { useNow } from "../app/hooks";
import { formatPairCode, isCompletePairCode, retrySeconds, waitPhrase } from "../app/pair-code";
import { takeReturn } from "../app/pair-return";
import { Button } from "../components/button";
import { Input } from "../components/input";
import { Panel } from "../components/panel";
import { SectionHeader } from "../components/section-header";
import { apiStore, getAdapter, isApiError } from "../lib/api";
import { consumeParam, readParams } from "../lib/params";

type Failure = { kind: "wrong" } | { kind: "limited"; seconds: number } | { kind: "other"; text: string };

function failureFor(err: unknown): Failure {
  if (!isApiError(err)) return { kind: "other", text: "Pairing failed. Try again." };
  if (err.status === 429 || err.code === "rate_limited") return { kind: "limited", seconds: retrySeconds(err.problem.detail) };
  if (err.status === 401 || err.unauthenticated) return { kind: "wrong" };
  if (err.code === "network") return { kind: "other", text: "Could not reach the node. Check that it is running and try again." };
  return { kind: "other", text: err.problem.detail || err.problem.title };
}

export default function PairView() {
  // The QR code opens `#/pair?code=xxxx-xxxx`: prefill the field, never submit on the visitor's behalf.
  const [code, setCode] = useState(() => formatPairCode(readParams(window.location.search, window.location.hash).get("code") ?? ""));
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [blockedUntil, setBlockedUntil] = useState(0);
  const now = useNow(blockedUntil > Date.now());
  const wait = Math.max(0, Math.ceil((blockedUntil - now) / 1000));

  // Strip `?code=` from the address bar, and send loopback (or already paired) visitors home.
  useEffect(() => {
    let cancelled = false;
    consumeParam("code");
    void (async () => {
      try {
        const adapter = await getAdapter();
        const session = await adapter.auth.session();
        if (cancelled || !(session.loopback || session.paired)) return;
        // Clear a stale "needs pairing" flag first, or the auth gate would bounce straight back here.
        await apiStore.refreshAll();
        if (!cancelled && !apiStore.getState().meta.needsPair) navigate(takeReturn(), { replace: true });
      } catch {
        /* node unreachable: stay on the form */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const canSubmit = isCompletePairCode(code) && wait === 0 && !busy;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setFailure(null);
    try {
      const adapter = await getAdapter();
      await adapter.auth.pair(code);
      // New cookie: restart the live stream now (it was paused while we were unpaired), reload every
      // snapshot, and leave only once the store agrees we are in (otherwise the auth gate sends us back).
      await apiStore.attach(adapter);
      if (apiStore.getState().meta.needsPair) {
        setFailure({ kind: "other", text: "Paired, but the node still refuses this browser. Check that cookies are enabled for this address." });
        return;
      }
      navigate(takeReturn(), { replace: true });
    } catch (err) {
      const f = failureFor(err);
      if (f.kind === "limited") setBlockedUntil(Date.now() + f.seconds * 1000);
      setFailure(f);
    } finally {
      setBusy(false);
    }
  };

  // The alert text never changes while it is on screen (a live region that rewrites itself every second
  // is read out every second); the countdown beside it is not live.
  let message: string | null = null;
  if (failure?.kind === "wrong") message = "Wrong code. Check the code shown on the node and try again.";
  else if (failure?.kind === "limited" && wait > 0) message = `Too many attempts. Wait ${waitPhrase(failure.seconds)}.`;
  else if (failure?.kind === "other") message = failure.text;
  const host = typeof window === "undefined" ? "" : window.location.host;

  return (
    <main id="main" className="mx-auto flex min-h-dvh w-full max-w-md flex-col justify-center gap-6 px-4 py-10">
      <SectionHeader level={1} kicker="pair this browser" title="Pair" />
      <Panel title="pair">
        <p className="font-mono text-xs leading-relaxed text-muted-foreground">
          On the node run <code className="border border-border px-1 text-foreground">111 dashboard</code> and type the pair code shown there.
        </p>
        {host ? (
          <p className="mt-2 font-mono text-xs leading-relaxed text-muted-foreground">
            This browser is pairing with the node at <span className="break-all text-foreground">{host}</span>.
          </p>
        ) : null}
        <form onSubmit={submit} noValidate className="mt-4 flex flex-col gap-3">
          <label htmlFor="pair-code" className="text-[10px] uppercase tracking-widest text-muted-foreground">
            Pair code
          </label>
          <Input
            id="pair-code"
            value={code}
            onChange={(e) => {
              setCode(formatPairCode(e.target.value));
              if (failure?.kind !== "limited") setFailure(null);
            }}
            inputMode="text"
            autoComplete="off"
            autoCapitalize="characters"
            autoCorrect="off"
            spellCheck={false}
            autoFocus
            placeholder="xxxx-xxxx"
            aria-invalid={failure?.kind === "wrong" || undefined}
            aria-describedby={message ? "pair-message" : undefined}
            className="tracking-widest"
          />
          {message ? (
            <p id="pair-message" role="alert" className="font-mono text-xs text-foreground">
              {message}
            </p>
          ) : null}
          {failure?.kind === "limited" && wait > 0 ? (
            <p aria-live="off" className="font-mono text-xs text-muted-foreground">
              You can try again in {wait}s.
            </p>
          ) : null}
          <Button type="submit" variant="primary" loading={busy} disabled={!canSubmit && !busy}>
            Pair
          </Button>
        </form>
      </Panel>
    </main>
  );
}
