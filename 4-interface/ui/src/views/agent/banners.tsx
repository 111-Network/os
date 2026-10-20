// The strips around the transcript: prepared-prompt banner, the no-model offer card, and the
// one-line state notice.
import { useEffect, useState } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { Button } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Disclosure } from "../../components/disclosure";
import { apiStore, getAdapter } from "../../lib/api";
import { reasonClause, type AgentStateInfo } from "./state";

export interface PreparedBannerProps {
  from: string;
  text: string;
  /** The agent is unavailable: the prompt cannot be sent, so offer it for copying. */
  unavailable: boolean;
  onDismiss: () => void;
}

export function PreparedBanner({ from, text, unavailable, onDismiss }: PreparedBannerProps) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-border px-4 py-2 text-xs text-muted-foreground">
      {unavailable ? (
        <>
          <span className="min-w-0 flex-1">the agent is unavailable &mdash; copy this request for when it is back</span>
          <CopyButton text={text} label="copy the request" ariaLabel="Copy the prepared request" />
        </>
      ) : (
        <span className="min-w-0 flex-1">prepared from: {from} &mdash; edit, then send</span>
      )}
      <Button size="sm" onClick={onDismiss} aria-label="Dismiss the prepared-prompt note">
        dismiss
      </Button>
    </div>
  );
}

export interface OfferCardProps {
  state: AgentStateInfo;
  /** `status.auth.loopback`; undefined until the first status arrives. */
  loopback: boolean | undefined;
}

/** The offer is open the first time this page load shows it, then folds to one line on later visits. */
let offerSeen = false;

/** Shown when the model is unset: the standard-tier offer, acceptable on the node itself only. */
export function OfferCard({ state, loopback }: OfferCardProps) {
  const [firstView] = useState(() => !offerSeen);
  useEffect(() => {
    offerSeen = true;
  }, []);
  const [busy, setBusy] = useState(false);
  const [accepted, setAccepted] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const accept = async () => {
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      await adapter.patchSettings({ offer: "accepted" });
      setAccepted(true);
      void apiStore.refreshAll();
    } catch (err) {
      if (!redirectIfUnauthenticated(err)) setError(err instanceof Error ? err.message : "The offer could not be accepted.");
    } finally {
      setBusy(false);
    }
  };

  const alreadyAccepted = accepted || state.offer === "accepted";
  const canOffer = state.modelCanRun && state.offer !== null;
  const lanBlocked = loopback !== true;

  return (
    <div className="shrink-0 border-b border-border px-4" data-testid="offer-card">
      <Disclosure summary={<span className="uppercase tracking-widest">no model yet</span>} defaultOpen={firstView} className="max-h-[40dvh] overflow-y-auto">
        <p className="max-w-prose text-sm text-foreground">{state.reason ?? "No local model is installed yet."}</p>
        {alreadyAccepted ? (
          <p role="status" className="mt-2 max-w-prose text-xs text-muted-foreground">
            You accepted the standard tier. The model is being prepared; chat works once it is running.
          </p>
        ) : canOffer ? (
          <div className="mt-3 flex flex-wrap items-center gap-3">
            <Button size="sm" variant="primary" loading={busy} disabled={lanBlocked} aria-describedby={lanBlocked ? "offer-lan-note" : undefined} onClick={() => void accept()}>
              accept the standard tier
            </Button>
            {lanBlocked ? (
              <span id="offer-lan-note" className="text-xs text-muted-foreground">
                available on the node itself
              </span>
            ) : (
              <span className="text-xs text-muted-foreground">Starts a download in the background. You can keep chatting once the model is running.</span>
            )}
          </div>
        ) : (
          <p className="mt-2 max-w-prose text-xs text-muted-foreground">This device has no local model on offer. Messages you type are queued until an agent is available.</p>
        )}
        {error ? (
          <p role="alert" className="mt-2 text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </Disclosure>
    </div>
  );
}

/** One plain line for starting / unavailable, so the reason is visible without opening Services. */
export function StateNotice({ state }: { state: AgentStateInfo }) {
  if (state.kind !== "unavailable" && state.kind !== "starting") return null;
  const why = reasonClause(state.reason);
  return (
    <p className="border-b border-border px-4 py-2 text-xs text-muted-foreground">
      {state.kind === "unavailable" ? "The agent is unavailable" : "The agent is starting"}
      {why ? `: ${why}` : ""}. Messages you send wait until it is ready.
    </p>
  );
}
