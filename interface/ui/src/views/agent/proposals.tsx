// Open proposals as inline cards above the input (spec 6.1): summary, kind, created time, the body
// behind a disclosure, and Mark accepted / Dismiss / Why?. Decisions are recorded for Layer 9 to
// pick up; nothing is applied from here.
import { useState } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { formatAgo } from "../../app/format";
import { useNow } from "../../app/hooks";
import { announce } from "../../app/LiveRegion";
import { Badge } from "../../components/badge";
import { Button } from "../../components/button";
import { Disclosure } from "../../components/disclosure";
import { apiStore, getAdapter, isApiError } from "../../lib/api";
import type { Decision, Proposal } from "../../lib/api/sse.gen";
import { ProposalBody } from "./proposal-body";

function createdText(iso: string, now: number): string {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : formatAgo((now - t) / 1000);
}

interface CardProps {
  proposal: Proposal;
  now: number;
  onWhy: (p: Proposal) => void;
  onDecided: () => void;
}

function ProposalCard({ proposal, now, onWhy, onDecided }: CardProps) {
  const [busy, setBusy] = useState<Decision | null>(null);
  const [error, setError] = useState<string | null>(null);

  const decide = async (decision: Decision) => {
    setBusy(decision);
    setError(null);
    try {
      const adapter = await getAdapter();
      await adapter.decide(proposal.id, decision);
      announce(decision === "accepted" ? "proposal marked accepted" : "proposal dismissed");
      onDecided();
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      // Decided elsewhere in the meantime: the refresh below removes the card.
      if (!(isApiError(err) && err.code === "already_decided")) setError(err instanceof Error ? err.message : "The decision could not be saved.");
    } finally {
      setBusy(null);
      void apiStore.refreshProposals();
    }
  };

  return (
    <li className="border border-border p-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <p className="min-w-0 flex-1 break-words text-sm text-foreground">{proposal.title}</p>
        <Badge>{proposal.kind}</Badge>
        <time dateTime={proposal.created_at} title={proposal.created_at} className="text-[10px] text-muted-foreground">
          {createdText(proposal.created_at, now)}
        </time>
      </div>
      <Disclosure summary="details" className="mt-1">
        <ProposalBody body={proposal.body} />
      </Disclosure>
      <div className="mt-1 flex flex-wrap gap-2">
        <Button size="sm" variant="primary" arrow={false} loading={busy === "accepted"} disabled={busy !== null} onClick={() => void decide("accepted")}>
          Mark accepted
        </Button>
        <Button size="sm" loading={busy === "dismissed"} disabled={busy !== null} onClick={() => void decide("dismissed")}>
          Dismiss
        </Button>
        <Button size="sm" disabled={busy !== null} onClick={() => onWhy(proposal)}>
          Why?
        </Button>
      </div>
      {error ? (
        <p role="alert" className="mt-2 text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </li>
  );
}

export interface ProposalCardsProps {
  /** Open proposals, newest first. */
  proposals: Proposal[];
  onWhy: (p: Proposal) => void;
  /** A decision was saved (the card is about to disappear): move focus somewhere sensible. */
  onDecided: () => void;
}

export function ProposalCards({ proposals, onWhy, onDecided }: ProposalCardsProps) {
  const now = useNow(proposals.length > 0, 30_000);
  if (proposals.length === 0) return null;
  return (
    <section aria-label="Open proposals" className="max-h-[35%] shrink-0 overflow-y-auto border-t border-border bg-secondary/20 p-3">
      <p className="mb-2 text-[10px] uppercase tracking-widest text-muted-foreground">
        {proposals.length === 1 ? "1 proposal needs you" : `${proposals.length} proposals need you`}
      </p>
      <ul className="flex flex-col gap-2">
        {proposals.map((p) => (
          <ProposalCard key={p.id} proposal={p} now={now} onWhy={onWhy} onDecided={onDecided} />
        ))}
      </ul>
      <p className="mt-2 text-xs text-muted-foreground">Applying accepted proposals arrives with Layer 9 (Repairs); for now follow the steps or ask the agent.</p>
    </section>
  );
}
