// Agent view (spec 6.1): a plain chat box to the node's local AI. The conversation itself lives in
// `chatSession` (module level), so this component is only a window onto it.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { announce } from "../../app/LiveRegion";
import { Button } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { JsonBlock } from "../../components/json-block";
import { Panel } from "../../components/panel";
import { StatusDot } from "../../components/status-dot";
import { useActivity, useProposals, useServices, useStatus } from "../../lib/api/hooks";
import { openProposalsOf } from "../../lib/api/store";
import { consumeParam } from "../../lib/params";
import { Adv } from "../../lib/mode";
import { buildPrompt, takePrompt } from "../../lib/prompts";
import { OfferCard, PreparedBanner, StateNotice } from "./banners";
import { Composer } from "./composer";
import { chatSession as session } from "./instance";
import { Greeting, QueuedList, Transcript, type Starter } from "./messages";
import { ProposalCards } from "./proposals";
import { agentState, queuedNotice, whyPrompt } from "./state";

const FOLLOW_SLACK_PX = 24;
const RAW_EVENTS = 50;

export default function AgentView() {
  const services = useServices();
  const status = useStatus();
  const proposals = useProposals();
  const activity = useActivity();
  const snap = useSyncExternalStore(session.subscribe, session.getSnapshot, session.getSnapshot);

  const state = useMemo(() => agentState(services, status), [services, status]);
  const open = useMemo(() => openProposalsOf(proposals), [proposals]);

  // --- tell the session what the agent can do right now --------------------------------------
  const notice = queuedNotice(state);
  useEffect(() => {
    session.setServing(state.canSend, notice);
  }, [state.canSend, notice]);

  // --- mount: history, and a prompt handed over from another page ------------------------------
  useEffect(() => {
    const take = () => {
      const id = consumeParam("prompt");
      if (!id) return;
      const staged = takePrompt(id);
      if (staged) session.stage(staged.text, staged.from);
    };
    take();
    window.addEventListener("hashchange", take);
    void session.loadHistory();
    return () => window.removeEventListener("hashchange", take);
  }, []);

  // --- follow the newest message unless the reader scrolled up --------------------------------
  const listRef = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const onScroll = () => {
    const el = listRef.current;
    if (el) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK_PX;
  };
  useLayoutEffect(() => {
    const el = listRef.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [snap.rows, snap.live, snap.queued, snap.history]);
  const follow = useCallback(() => {
    pinned.current = true;
  }, []);

  // --- tell screen readers about a new proposal (not the ones already there at load) ----------
  const knownProposals = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (proposals === null) return;
    const ids = new Set(open.map((p) => p.id));
    const known = knownProposals.current;
    if (known) {
      const fresh = open.find((p) => !known.has(p.id));
      if (fresh) announce(`New proposal: ${fresh.title}`);
    }
    knownProposals.current = ids;
  }, [proposals, open]);

  const stage = useCallback((text: string) => session.stage(text), []);
  const starters: Starter[] = useMemo(
    () => [
      { label: "check this node", text: buildPrompt("check", { kind: "node" }, { status }) },
      { label: "set up the LoRa radio", text: "Set up the LoRa radio on this node and test it." },
      { label: "who is nearby?", text: "Who is nearby on the mesh? Tell me which other nodes this one has heard." },
    ],
    [status],
  );

  const focusList = useCallback(() => listRef.current?.focus(), []);

  const empty = snap.rows.length === 0 && !snap.live && snap.queued.length === 0;
  const intelligence = status?.intelligence;

  const chip = (
    <span className="inline-flex max-w-[12.5rem] flex-wrap items-center justify-end gap-x-3 gap-y-1 sm:max-w-none">
      <span className="inline-flex items-center gap-2" data-testid="agent-chip">
        <StatusDot state={state.dot} label={false} name="Agent" />
        <span className="text-foreground">{state.label}</span>
      </span>
      <Button
        size="sm"
        aria-label="New conversation"
        {...(snap.rows.length > 0 || snap.queued.length > 0 ? { confirm: "new conversation" } : {})}
        onClick={() => session.newConversation()}
      >
        <span className="sm:hidden">new</span>
        <span className="hidden sm:inline">new conversation</span>
      </Button>
    </span>
  );

  return (
    <Panel
      title="agent"
      suffix="chat"
      chromeEnd={chip}
      flush
      className="flex h-[calc(100dvh-7rem)] min-h-[26rem] flex-col"
      bodyClassName="flex min-h-0 flex-1 flex-col font-mono text-sm leading-relaxed"
    >
      <Adv>
        <div className="flex flex-wrap items-center gap-x-5 gap-y-1 border-b border-border px-4 py-1 text-xs text-muted-foreground">
          <span>model: {intelligence?.model_id ?? "none"}</span>
          <span>serving: {intelligence?.agent_state ?? "unknown"}</span>
          <span className="inline-flex items-center gap-2">
            <span className="break-all">conversation: {snap.conversationId}</span>
            <CopyButton text={snap.conversationId} label="copy" ariaLabel="Copy conversation id" />
          </span>
        </div>
      </Adv>

      {snap.prepared ? (
        <PreparedBanner from={snap.prepared.from} text={snap.prepared.text} unavailable={state.kind === "unavailable"} onDismiss={() => session.dismissPrepared()} />
      ) : null}
      {state.kind === "no-model" ? <OfferCard state={state} loopback={status?.auth.loopback} /> : null}
      {snap.queued.length === 0 ? <StateNotice state={state} /> : null}

      <div
        ref={listRef}
        role="log"
        aria-live="off"
        aria-label="Conversation"
        tabIndex={0}
        onScroll={onScroll}
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 py-4 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none"
      >
        {snap.history === "loading" && empty ? <p className="text-xs text-muted-foreground">loading conversation</p> : null}
        {snap.history === "error" ? <p className="text-xs text-muted-foreground">The earlier messages of this conversation could not be loaded.</p> : null}
        {empty && snap.history !== "loading" ? <Greeting starters={starters} onStage={stage} /> : null}
        <Transcript rows={snap.rows} live={snap.live} />
        <QueuedList queued={snap.queued} onRemove={(id) => session.removeQueued(id)} onRetry={() => session.retryQueued()} />
      </div>

      {snap.notice ? (
        <p role="status" className="border-t border-border px-4 py-2 text-xs text-muted-foreground">
          {snap.notice}
        </p>
      ) : null}

      <ProposalCards proposals={open} onWhy={(p) => session.stage(whyPrompt(p, status))} onDecided={focusList} />

      <Adv>
        <div className="border-t border-border px-4">
          <JsonBlock summary={`raw events (last ${Math.min(activity.length, RAW_EVENTS)})`} value={activity.slice(0, RAW_EVENTS)} />
        </div>
      </Adv>

      <Composer session={session} streaming={snap.live !== null} draftSeq={snap.draftSeq} onSubmitted={follow} />
    </Panel>
  );
}
