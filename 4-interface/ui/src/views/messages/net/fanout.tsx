// "Send to networks…" (os/6-transport/README.md §9): one text, copied to any conversation of any enabled network, a Reticulum
// recipient list, or a 111 contact. A fan-out is a fresh send group, never a 111 message exported. The preview (no queue write, no
// transmit, no log of the text) shows per target the bytes against the cap, the parts if split, the airtime estimate (labelled one),
// the region and licence facts, and any refusal ("312 B > 232 B per packet"). Size mode is per target and per send. Sending is
// atomic: a refused target answers 409 with a per-target report and nothing is queued.
import { useEffect, useMemo, useState } from "react";
import { redirectIfUnauthenticated } from "../../../app/AuthGate";
import { announce } from "../../../app/LiveRegion";
import { useNow } from "../../../app/hooks";
import { hrefTo } from "../../../app/location";
import { Badge } from "../../../components/badge";
import { Button, ButtonLink } from "../../../components/button";
import { Empty } from "../../../components/empty";
import { FIELD_CLASS } from "../../../components/input";
import { Panel } from "../../../components/panel";
import { SectionHeader } from "../../../components/section-header";
import { Textarea } from "../../../components/textarea";
import { getAdapter, isApiError } from "../../../lib/api";
import { confirmGate, refusalsOf } from "../../../lib/api/bridge";
import { contactName } from "../../../lib/api/contacts";
import { useContacts, useConversationsOf, useForeignThread, useNetworks } from "../../../lib/api/hooks";
import { enabledNetworks, groupParts } from "../../../lib/api/networks";
import type { Contact, FanoutPreview, FanoutResult, FanoutTarget, ForeignConversation, Network, PlanRefusal, SendItem, SendMode } from "../../../lib/api/types";
import { MAX_COMPOSE_CHARS, errorSentence } from "../model";
import { useConsentFlow } from "./consent";
import { NetChip } from "./glyph";
import { PREVIEW_ESTIMATE_NOTE, SIZE_MODE_HELP, airtimeText, convTitle, kindWord, planSizeText, targetKey, utf8Length } from "./model";
import { ForeignRow, useRowActions } from "./thread";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

interface Picked {
  target: FanoutTarget;
  key: string;
  label: string;
  chip: string;
  splittable: boolean;
}

function toRequestTarget(p: Picked, modes: Record<string, SendMode>): FanoutTarget {
  return "conv" in p.target ? { network: p.target.network, conv: p.target.conv, size_mode: modes[p.key] ?? "refuse" } : p.target;
}

function SentForeign({ item, network }: { item: SendItem; network: Network | null }) {
  const t = item.target;
  const net = "network" in t ? t.network : "";
  const conv = "conv" in t ? t.conv : "";
  const slot = useForeignThread(net, conv);
  const { retry, cancel, error } = useRowActions(net, conv);
  const now = useNow(true, 30_000);
  const ids = item.ids ?? [item.id];
  const rows = (slot.page?.messages ?? []).filter((m) => ids.includes(m.id));
  if (rows.length === 0) {
    return (
      <li className="px-4 py-3 font-mono text-xs text-muted-foreground" data-testid={`sent-${item.id}`}>
        {slot.state === "missing" || slot.state === "ready" ? "It is no longer in the conversation (cancelled or removed)." : "Queued. Loading its state."}
      </li>
    );
  }
  const it = groupParts(rows)[0]!;
  return (
    <li className="flex flex-col gap-1 px-4 py-3" data-testid={`sent-${item.id}`}>
      <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        {network?.label ?? net} · {slot.page ? convTitle(slot.page.conversation) : conv}
      </p>
      <ul>
        <ForeignRow item={it} network={network} now={now} actions={{ retry, cancel }} />
      </ul>
      {error ? (
        <p role="alert" className="font-mono text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </li>
  );
}

export function PlanLines({ plan, onSplit, onConfirm, splittable }: { plan: FanoutPreview["items"][number]["plan"]; onSplit: () => void; onConfirm: (r: PlanRefusal) => void; splittable: boolean }) {
  const air = airtimeText(plan);
  return (
    <div className="flex flex-col gap-1" data-testid="plan">
      <p className="font-mono text-xs text-foreground" data-testid="plan-size">
        {planSizeText(plan)}
        {plan.parts > 1 ? " (each part counts its (k/n) prefix)" : ""}
      </p>
      {air ? <p className={TEXT}>airtime: {air}</p> : null}
      {plan.warnings.map((w) => (
        <p key={w.code} className={TEXT} data-testid={`warn-${w.code}`}>
          {w.text}
        </p>
      ))}
      {plan.refusal ? (
        <div className="flex flex-col gap-2 border border-foreground p-2" role="alert" data-testid="refusal">
          <p className="font-mono text-xs text-foreground">
            Refused: {plan.refusal.detail}
          </p>
          <div className="flex flex-wrap gap-2">
            {plan.refusal.code === "oversize" && splittable ? (
              <Button size="sm" onClick={onSplit}>
                split this one into parts
              </Button>
            ) : null}
            {plan.refusal.consent ? (
              <Button size="sm" onClick={() => onConfirm(plan.refusal!)}>
                read and confirm…
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

export default function FanoutView() {
  const networks = useNetworks();
  const enabled = enabledNetworks(networks);
  const convsBy = useConversationsOf(enabled.map((n) => n.id));
  const contacts = useContacts();
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<Record<string, Picked>>({});
  const [modes, setModes] = useState<Record<string, SendMode>>({});
  const [preview, setPreview] = useState<FanoutPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [rev, setRev] = useState(0);
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState<FanoutResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { dialog, attempt } = useConsentFlow();

  const chosen = Object.values(picked);
  const request = useMemo(() => ({ text, targets: chosen.map((p) => toRequestTarget(p, modes)) }), [text, picked, modes]);
  const requestKey = JSON.stringify(request);

  // The preview follows the text and the targets. It writes nothing and the text is not logged.
  useEffect(() => {
    if (text.trim() === "" || request.targets.length === 0) {
      setPreview(null);
      setPreviewError(null);
      return;
    }
    let live = true;
    const handle = setTimeout(() => {
      void (async () => {
        try {
          const adapter = await getAdapter();
          const p = await adapter.networks.fanoutPreview(request);
          if (live) {
            setPreview(p);
            setPreviewError(null);
          }
        } catch (err) {
          if (!live || redirectIfUnauthenticated(err)) return;
          setPreview(null);
          setPreviewError(errorSentence(err, "The preview could not be made."));
        }
      })();
    }, 250);
    return () => {
      live = false;
      clearTimeout(handle);
    };
  }, [requestKey, rev]);

  const toggle = (p: Picked) =>
    setPicked((cur) => {
      const next = { ...cur };
      if (next[p.key]) delete next[p.key];
      else next[p.key] = p;
      return next;
    });

  const byKey = new Map(chosen.map((p) => [p.key, p] as const));
  const previewFor = (p: Picked, i: number) => (preview && targetKey(preview.items[i]?.target ?? p.target) === p.key ? preview.items[i] : undefined);

  const confirmRefusal = async (r: PlanRefusal) => {
    if (!r.consent) return;
    const adapter = await getAdapter();
    const out = await attempt(async (hash, c) => {
      if (hash && c) await confirmGate(adapter, c, hash);
    }, r.consent);
    if (out.ok) {
      announce("Confirmed. Checking the send again.");
      setRev((n) => n + 1);
    } else if (!out.cancelled && !redirectIfUnauthenticated(out.error)) setError(errorSentence(out.error, "The confirmation could not be saved."));
  };

  const send = async () => {
    if (sending) return;
    setSending(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      const result = await adapter.networks.fanout(request);
      setSent(result);
      setText("");
      setPicked({});
      announce(`Queued ${result.items.length} ${result.items.length === 1 ? "send" : "sends"}. Each has its own state.`);
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      const refused = isApiError(err) ? refusalsOf(err) : [];
      if (refused.length > 0) {
        setError(`Nothing was queued. ${refused.length === 1 ? "1 target was" : `${refused.length} targets were`} refused: ${refused.map((r) => r.detail).join("; ")}`);
        setRev((n) => n + 1);
      } else setError(errorSentence(err, "The send could not be queued."));
    } finally {
      setSending(false);
    }
  };

  const contactList = (contacts ?? []).filter((c) => !c.request);
  const nothing = enabled.length === 0 && contactList.length === 0;
  const bytes = utf8Length(text);
  const ready = text.trim() !== "" && chosen.length > 0 && preview?.ok === true;

  return (
    <div className="flex flex-col gap-6" data-testid="fanout">
      <SectionHeader
        kicker="messages"
        title="Send to networks"
        level={1}
        end={
          <ButtonLink href={hrefTo("/messages")} size="sm">
            All messages
          </ButtonLink>
        }
      />
      <p className="max-w-prose font-mono text-sm leading-relaxed text-foreground">
        One text, copied to each target you pick. It is a copy: nothing is exported from a 111 conversation, and a 111 contact still gets an ordinary sealed message. Other networks can read what you send there, and each target has its own state.
      </p>
      {nothing ? (
        <Empty kicker="nothing to send to" title="No network is on">
          Turn a network on in Settings, Transport, or add a contact, then come back.
        </Empty>
      ) : (
        <>
          <Panel title="text" suffix="a copy">
            <label className="flex flex-col gap-2">
              <span className="font-mono text-[10px] uppercase tracking-widest text-foreground">what to send</span>
              <Textarea value={text} rows={4} maxLength={MAX_COMPOSE_CHARS} autoComplete="off" placeholder="Write the text once" onChange={(e) => setText(e.target.value)} data-testid="fanout-text" />
            </label>
            <p className={`mt-2 ${TEXT}`}>{bytes} B as UTF-8.</p>
          </Panel>

          <Panel title="targets" suffix={`${chosen.length} picked`}>
            <div className="flex flex-col gap-4">
              {enabled.map((n) => (
                <fieldset key={n.id} className="flex flex-col gap-1" data-testid={`targets-${n.id}`}>
                  <legend className="flex items-center gap-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                    <NetChip chip={n.chip} /> {n.label}
                  </legend>
                  {(convsBy[n.id] ?? []).map((c) => (
                    <TargetBox key={c.id} n={n} c={c} picked={byKey.has(`f:${c.id}`)} mode={modes[`f:${c.id}`] ?? "refuse"} onToggle={toggle} onMode={(m) => setModes((cur) => ({ ...cur, [`f:${c.id}`]: m }))} />
                  ))}
                  {(convsBy[n.id] ?? []).length === 0 ? <p className={TEXT}>No conversations yet.</p> : null}
                </fieldset>
              ))}
              {contactList.length > 0 ? (
                <fieldset className="flex flex-col gap-1" data-testid="targets-111">
                  <legend className="flex items-center gap-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                    <NetChip chip="111" /> contacts, sealed
                  </legend>
                  {contactList.map((c) => (
                    <ContactBox key={c.node_id} c={c} picked={byKey.has(`c:${c.node_id}`)} onToggle={toggle} />
                  ))}
                </fieldset>
              ) : null}
            </div>
          </Panel>

          <Panel title="preview" suffix="no queue write, no transmit">
            {previewError ? (
              <p role="alert" className="font-mono text-xs text-destructive">
                {previewError}
              </p>
            ) : chosen.length === 0 || text.trim() === "" ? (
              <p className={TEXT}>Write the text and pick at least one target to see what each would do.</p>
            ) : preview === null ? (
              <p role="status" className={TEXT}>
                Checking…
              </p>
            ) : (
              <ul className="flex flex-col gap-4" data-testid="preview">
                {chosen.map((p, i) => {
                  const it = previewFor(p, i);
                  return (
                    <li key={p.key} className="flex flex-col gap-1 border-b border-border pb-3 last:border-b-0" data-testid={`preview-${p.key}`}>
                      <p className="flex flex-wrap items-center gap-2 font-mono text-sm text-foreground">
                        <NetChip chip={p.chip} /> <span dir="auto" className="min-w-0 break-words [unicode-bidi:isolate]">{p.label}</span>
                      </p>
                      {it ? (
                        <PlanLines
                          plan={it.plan}
                          splittable={p.splittable}
                          onSplit={() => setModes((cur) => ({ ...cur, [p.key]: "split" }))}
                          onConfirm={(r) => void confirmRefusal(r)}
                        />
                      ) : (
                        <p className={TEXT}>…</p>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
            <p className={`mt-3 ${TEXT}`}>{PREVIEW_ESTIMATE_NOTE}</p>
          </Panel>

          {dialog}
          <div className="flex flex-wrap items-center gap-3">
            <Button variant="primary" loading={sending} aria-disabled={!ready || undefined} onClick={() => void send()} data-testid="fanout-send">
              send to {chosen.length === 1 ? "1 target" : `${chosen.length} targets`}
            </Button>
            {!ready && chosen.length > 0 && text.trim() !== "" && preview && !preview.ok ? <p className={TEXT}>Fix the refused targets first: nothing is sent unless every target can take it.</p> : null}
          </div>
          {error ? (
            <p role="alert" className="max-w-prose font-mono text-xs text-destructive" data-testid="fanout-error">
              {error}
            </p>
          ) : null}
        </>
      )}

      {sent ? (
        <Panel title="sent" suffix={sent.group_id} flush>
          <ul className="divide-y divide-border" data-testid="sent-items">
            {sent.items.map((it) =>
              "node_id" in it.target ? (
                <li key={it.id} className="px-4 py-3 font-mono text-xs text-muted-foreground" data-testid={`sent-${it.id}`}>
                  Sealed 111 message queued for a contact. Its state is in{" "}
                  <a className="underline underline-offset-4" href={hrefTo(`/messages/${it.target.node_id}`)}>
                    that conversation
                  </a>
                  .
                </li>
              ) : (
                <SentForeign key={it.id} item={it} network={(networks ?? []).find((n) => "network" in it.target && n.id === it.target.network) ?? null} />
              ),
            )}
          </ul>
        </Panel>
      ) : null}
    </div>
  );
}

function TargetBox({ n, c, picked, mode, onToggle, onMode }: { n: Network; c: ForeignConversation; picked: boolean; mode: SendMode; onToggle: (p: Picked) => void; onMode: (m: SendMode) => void }) {
  const p: Picked = { target: { network: n.id, conv: c.id }, key: `f:${c.id}`, label: convTitle(c), chip: n.chip, splittable: n.caps.splittable };
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 py-1">
      <label className="flex min-h-11 min-w-0 items-center gap-3 font-mono text-sm text-foreground">
        <input type="checkbox" checked={picked} onChange={() => onToggle(p)} className="h-4 w-4 shrink-0" />
        <span dir="auto" className="min-w-0 break-words [unicode-bidi:isolate]">
          {convTitle(c)}
        </span>
        <Badge>{kindWord(c.kind)}</Badge>
      </label>
      {picked && n.caps.splittable ? (
        <label className="flex items-center gap-2 font-mono text-xs text-muted-foreground">
          <span>too long:</span>
          <select className={`${FIELD_CLASS} !min-h-9 !w-auto !py-1`} value={mode} onChange={(e) => onMode(e.target.value === "split" ? "split" : "refuse")} aria-describedby="mode-help" aria-label={`If the text is too long for ${convTitle(c)}`}>
            <option value="refuse">refuse</option>
            <option value="split">split</option>
          </select>
        </label>
      ) : null}
      {picked && n.caps.splittable ? (
        <p id="mode-help" className="basis-full font-mono text-[11px] text-muted-foreground">
          {SIZE_MODE_HELP[mode]}
        </p>
      ) : null}
    </div>
  );
}

function ContactBox({ c, picked, onToggle }: { c: Contact; picked: boolean; onToggle: (p: Picked) => void }) {
  const label = contactName(c);
  const p: Picked = { target: { node_id: c.node_id }, key: `c:${c.node_id}`, label, chip: "111", splittable: false };
  return (
    <label className="flex min-h-11 items-center gap-3 font-mono text-sm text-foreground">
      <input type="checkbox" checked={picked} onChange={() => onToggle(p)} className="h-4 w-4 shrink-0" />
      <span dir="auto" className="min-w-0 break-words [unicode-bidi:isolate]">
        {label}
      </span>
    </label>
  );
}
