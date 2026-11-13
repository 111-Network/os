// Settings, Relay & presence, Paired hubs (os/7-relay/README.md §4.1, §4.6, §12): this hub's pairing bundle as a QR and as text, the
// other hub's bundle scanned or pasted, the role and link pick with the complement rule, the `lan` mark, each pair's protection (`tls
// rpk`, `lan`, `plain (warned)`) and unpair. Pairing and unpairing are loopback-only: the node answers 403 and the page says so.
// An unencrypted pair on the open internet is the `pair_plain` consent: the node's words, then the call repeated with their hash.
import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { Badge } from "../../../components/badge";
import { Button } from "../../../components/button";
import { CopyButton } from "../../../components/copy-button";
import { Panel } from "../../../components/panel";
import { Segmented } from "../../../components/segmented";
import { Textarea } from "../../../components/textarea";
import { Toggle } from "../../../components/toggle";
import { useFabric } from "../../../lib/api/hooks";
import { plain } from "../../nearby/model";
import type { PairOffer, PairRole, PairVia, PairedHub, RelayStatus } from "../../../lib/api/types";
import { pairFacts, pairLinkState } from "../../services/fabric-model";
import { PeerText } from "../../services/shared";
import { announce } from "../../../app/LiveRegion";
import { FEEDBACK, Feedback, Row } from "../row";
import { ScanBox } from "../messaging";
import { OFFER_GONE_TEXT, offerStep } from "../messaging-model";
import { QrCanvas } from "../qr";
import { useRelayCall } from "./actions";
import { CallOutput } from "./call-view";
import {
  bundleSummary,
  COMPLEMENT_RULE,
  HUB_ONLY_TEXT,
  LAN_MARK_HELP,
  mismatchNote,
  offerNote,
  PAIR_STEPS,
  parsePairBundle,
  pickForBundle,
  type PairBundle,
  ROLE_MEANING,
  settlePick,
} from "./model";

const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";

function PickRow({ role, via, onPick }: { role: PairRole; via: PairVia; onPick: (role: PairRole, via: PairVia) => void }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-2">
        <Segmented<PairRole>
          label="Role"
          value={role}
          onChange={(r) => {
            const p = settlePick(r, via, "role");
            onPick(p.role, p.via);
          }}
          options={[
            { value: "peer", label: "peer" },
            { value: "leaf", label: "leaf" },
            { value: "gateway", label: "gateway" },
          ]}
        />
        <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`} data-testid="role-meaning">
          {ROLE_MEANING[role]}
        </p>
      </div>
      <Segmented<PairVia>
        label="Link"
        value={via}
        onChange={(v) => {
          const p = settlePick(role, v, "via");
          onPick(p.role, p.via);
        }}
        options={[
          { value: "tcp", label: "over the network" },
          { value: "carrier", label: "through the carrier" },
        ]}
      />
    </div>
  );
}

function ShowBundle({ gated }: { gated: boolean }) {
  const [role, setRole] = useState<PairRole>("peer");
  const [via, setVia] = useState<PairVia>("tcp");
  const [offer, setOffer] = useState<PairOffer | null>(null);
  /** The role or link was changed while a bundle was on screen: the bundle is hidden, but its code is still the node's until dismissed. */
  const [earlier, setEarlier] = useState(false);
  const [gone, setGone] = useState(false);
  const call = useRelayCall();
  // A code the node holds is in the fabric slice (Layer 5 `hub.offer_active`): one minted earlier, here or elsewhere, shows too.
  const fabric = useFabric();
  const outstanding = fabric?.hub.offer_active === true;
  const seen = useRef(false);
  const dismissing = useRef(false);

  // A code on screen that the node no longer holds (a member used it, it was replaced or dismissed elsewhere) is dead: stop showing it.
  useEffect(() => {
    const step = offerStep(offer !== null, seen.current, outstanding, dismissing.current);
    seen.current = step.seen;
    if (!step.clear) return;
    setOffer(null);
    setEarlier(false);
    if (step.say) {
      setGone(true);
      announce(OFFER_GONE_TEXT);
    }
  }, [offer, outstanding]);

  const show = async () => {
    if (gated || call.state.pending) return;
    const made = await call.pairOffer({ role, via });
    if (made.ok) {
      setOffer(made.value);
      setEarlier(false);
      setGone(false);
    }
  };
  const dismiss = async () => {
    if (gated || call.state.pending) return;
    dismissing.current = true;
    const r = await call.dismissOffer({ done: "The code was dismissed." });
    dismissing.current = false;
    if (r.ok) {
      setOffer(null);
      setEarlier(false);
      setGone(false);
      announce("The code was dismissed.");
    }
  };
  const held = offer !== null || outstanding;
  // The "earlier" line is true only while the node still holds that code (or the fabric slice has not loaded yet): once a member
  // enrolled with it, or it was dismissed or replaced elsewhere, nothing is outstanding and the page says nothing.
  const note = offerNote(offer !== null, earlier && (fabric == null || outstanding), outstanding);
  return (
    <div className="flex flex-col gap-4" data-testid="pair-show">
      <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>Name the role this hub gives the other hub, then show your bundle for it to scan or paste. Each owner does this on their own hub.</p>
      <PickRow
        role={role}
        via={via}
        onPick={(r, v) => {
          setRole(r);
          setVia(v);
          if (offer) {
            setOffer(null);
            setEarlier(true);
          }
        }}
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant={held ? "default" : "primary"} arrow={false} loading={call.state.pending} aria-disabled={gated || undefined} onClick={() => void show()} data-testid="pair-offer">
          {held ? "Show a new bundle" : "Show my pairing bundle"}
        </Button>
        {held ? (
          <Button size="sm" confirm="Dismiss this code" aria-disabled={gated || call.state.pending || undefined} onClick={() => void dismiss()} data-testid="pair-dismiss">
            Dismiss this code
          </Button>
        ) : null}
      </div>
      <CallOutput call={call} />
      {note ? (
        <p role="status" className={`max-w-prose text-foreground ${FEEDBACK}`} data-testid="offer-outstanding">
          {note}
        </p>
      ) : null}
      {!note && !offer && gone ? (
        <p role="status" className={`max-w-prose text-foreground ${FEEDBACK}`} data-testid="offer-gone">
          {OFFER_GONE_TEXT}
        </p>
      ) : null}
      {offer ? (
        <div className="flex flex-col gap-3 border border-border p-3" data-testid="pair-bundle">
          <QrCanvas qr={offer.bundle.qr} label="Pairing QR code for this hub" />
          <code className="break-all font-mono text-xs leading-relaxed text-foreground" data-testid="pair-bundle-text">
            {offer.bundle.text}
          </code>
          <div className="flex flex-wrap items-center gap-2">
            <CopyButton text={offer.bundle.text} label="copy bundle" ariaLabel="Copy this hub's pairing bundle" />
          </div>
          <p className={`text-muted-foreground ${FEEDBACK}`}>
            The one-time code is inside the bundle: <span className="text-foreground">{offer.code}</span>. It works once and stays valid until you dismiss it or show a new bundle, which replaces it.
          </p>
        </div>
      ) : null}
    </div>
  );
}

function PairForm({ gated }: { gated: boolean }) {
  const id = useId();
  const [paste, setPaste] = useState("");
  const [role, setRole] = useState<PairRole>("peer");
  const [via, setVia] = useState<PairVia>("tcp");
  const [lan, setLan] = useState(false);
  const call = useRelayCall();
  const bundle = parsePairBundle(paste);
  /** The bundle the pick was last filled in from: a later edit of the same bundle never puts the pick back over the owner's own. */
  const picked = useRef<PairBundle | null>(null);

  /** Take a scanned or pasted text: a newly read bundle sets the matching pick. */
  const take = (text: string) => {
    setPaste(text);
    const b = parsePairBundle(text);
    const pick = pickForBundle(picked.current, b);
    if (b) picked.current = b;
    if (pick) {
      setRole(pick.role);
      setVia(pick.via);
      if (pick.via === "carrier") setLan(false);
    }
  };
  const note = bundle ? mismatchNote(bundle, role, via) : null;
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (gated || call.state.pending || !bundle) return;
    const made = await call.pair({ bundle: paste.trim(), role, via, lan: via === "tcp" && lan }, {
      done: (hub) => `Pairing started with ${plain(hub.label ?? "", 24) || "the other hub"}. It finishes in the background and shows below.`,
    });
    if (made.ok) {
      setPaste("");
      setLan(false);
      picked.current = null;
    }
  };
  return (
    <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)} noValidate data-testid="pair-scan">
      <div className="flex flex-col gap-2">
        <label htmlFor={`${id}-paste`} className={LABEL}>
          the other hub&apos;s bundle
        </label>
        <Textarea id={`${id}-paste`} rows={3} value={paste} autoComplete="off" spellCheck={false} placeholder="Paste what the other hub showed (it starts with 111PAIR:), or scan its QR" className="break-all" onChange={(e) => take(e.target.value)} />
        <ScanBox onText={take} />
      </div>
      {paste.trim() !== "" && !bundle ? <Feedback text="That is not a pairing bundle. It should start with 111PAIR: and end with a role and a link." problem /> : null}
      {bundle ? (
        <>
          <p role="status" className={`max-w-prose text-muted-foreground ${FEEDBACK}`} data-testid="bundle-read">
            <PeerText>{bundleSummary(bundle)}</PeerText>
          </p>
          <PickRow
            role={role}
            via={via}
            onPick={(r, v) => {
              setRole(r);
              setVia(v);
              if (v === "carrier") setLan(false);
            }}
          />
          <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{COMPLEMENT_RULE}</p>
          {note ? (
            <p role="status" className={`text-foreground ${FEEDBACK}`} data-testid="pair-mismatch">
              {note}
            </p>
          ) : null}
          {via === "tcp" ? (
            <div className="flex flex-col gap-2">
              <Toggle pressed={lan} onPressedChange={setLan}>
                <span>This link is a LAN or a tunnel I trust</span> <span aria-hidden="true">{lan ? "[ON]" : "[OFF]"}</span>
              </Toggle>
              <p className={`max-w-prose text-muted-foreground ${FEEDBACK}`}>{LAN_MARK_HELP}</p>
            </div>
          ) : null}
          <div>
            <Button type="submit" size="sm" variant="primary" arrow={false} loading={call.state.pending} aria-disabled={gated || undefined} data-testid="pair-submit">
              Pair with this hub
            </Button>
          </div>
        </>
      ) : null}
      <CallOutput call={call} />
    </form>
  );
}

function PairedRow({ hub }: { hub: PairedHub }) {
  const call = useRelayCall();
  const name = plain(hub.label, 24);
  return (
    <li className="flex flex-col gap-2 border-b border-border py-3 first:pt-0 last:border-b-0 last:pb-0" data-testid="paired-hub">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-sm text-foreground">
          <PeerText>{name}</PeerText>
        </span>
        <Badge className="border-foreground text-foreground" data-testid="pair-link-state">
          {pairLinkState(hub)}
        </Badge>
        {hub.lan ? <Badge data-testid="pair-lan-mark">lan</Badge> : null}
        <span className="flex-1" />
        <Button size="sm" confirm={`Unpair ${name}`} loading={call.state.pending} onClick={() => void call.unpair(hub.node_id)}>
          Unpair
        </Button>
      </div>
      <p className={`text-muted-foreground ${FEEDBACK}`}>{pairFacts(hub).join(" · ")}</p>
      {hub.tls === "plain" ? <p className={`text-foreground ${FEEDBACK}`}>This link is not encrypted. You accepted that when you paired.</p> : null}
      <CallOutput call={call} />
    </li>
  );
}

export function PairingSection({ relay, hubGate }: { relay: RelayStatus; hubGate: boolean }) {
  const pairs = Array.isArray(relay.paired_hubs) ? relay.paired_hubs : [];
  return (
    <Panel title="paired hubs" suffix="pair with another hub" flush>
      <Row id="paired-list" label="Paired hubs" help={hubGate ? HUB_ONLY_TEXT : "A pair forwards only when both hubs hold it. A link over the open internet is encrypted with each hub's own key; a link you marked as a LAN runs as it is."}>
        {pairs.length === 0 ? (
          <p className={`text-muted-foreground ${FEEDBACK}`} data-testid="no-pairs">
            No paired hubs.
          </p>
        ) : (
          <ul className="flex flex-col" data-testid="paired-list">
            {pairs.map((p) => (
              <PairedRow key={p.node_id} hub={p} />
            ))}
          </ul>
        )}
      </Row>
      <Row id="pair-show" label="Show my bundle" help={PAIR_STEPS}>
        <ShowBundle gated={hubGate} />
      </Row>
      <Row id="pair-scan" label="Pair with another hub" help="Scan or paste the bundle the other hub showed. The pick below is filled in from it: the other side of what it chose.">
        <PairForm gated={hubGate} />
      </Row>
    </Panel>
  );
}
