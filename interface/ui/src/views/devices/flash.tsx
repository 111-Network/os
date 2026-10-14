// The firmware panel on the device page (Layer 6, D17): a board with no usable firmware gets an offer to flash
// RNode firmware. The panel says what the board is, what would be written, where it comes from and what is
// erased; the confirmation is inline (never a modal, like Forget) and the user decides. Nothing here talks to
// the node except the final "Flash now".
//
//   FlashPanelView  presentational, one `step` at a time (the gallery renders it without a node)
//   FirmwareSection the device page's container: owns the step, the request and the focus
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import { useNow } from "../../app/hooks";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { Button } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Disclosure } from "../../components/disclosure";
import { Input } from "../../components/input";
import { Panel } from "../../components/panel";
import { SpecTable } from "../../components/spec-table";
import { apiStore, getAdapter, isApiError } from "../../lib/api";
import type { DeviceRecord, FlashOffer } from "../../lib/api/types";
import { cx, FOCUS_RING } from "../../lib/cx";
import { displayName, firmwareText } from "../../lib/devices";
import {
  EMPTY_CONFIRM,
  FLASH_CHECKS,
  FLASH_TITLE,
  confirmMissing,
  confirmValid,
  flashButtonLabel,
  flashErrorText,
  flashRequest,
  flashStalled,
  hasFirmwareNow,
  missingText,
  offerDisplay,
  type FlashConfirm,
} from "./flash-model";

const LABEL = "font-mono text-[10px] uppercase tracking-widest text-muted-foreground";
const NOTE = "max-w-prose font-mono text-[11px] leading-snug text-muted-foreground";

/** id of the opening button, so Cancel can hand the focus back to it. */
const OPEN_ID = "flash-open";

/** Square, monochrome checkbox: a filled inner square when ticked, a grey outline when not (44 px row below). */
const CHECKBOX = cx(
  "mt-0.5 h-5 w-5 shrink-0 cursor-pointer appearance-none border border-muted-foreground bg-background bg-clip-content p-[3px]",
  "checked:border-foreground checked:bg-foreground",
  FOCUS_RING,
);

export type FlashStep = "offer" | "confirm" | "started" | "stalled" | "done" | "gone";

type DeviceFacts = Pick<DeviceRecord, "alias" | "name" | "kind" | "transport" | "match" | "firmware">;

// ---------------------------------------------------------------- confirmation

export interface ConfirmStepProps {
  /** The name the user has to type: what the page shows as the device's name. */
  name: string;
  busy: boolean;
  error: string | null;
  onFlash: (c: FlashConfirm) => void;
  onCancel: () => void;
  /** Move the focus to the first checkbox when the step appears. Off in the gallery. */
  focusOnMount?: boolean;
}

/** The checklist, the typed name, "Flash now". Remounted (and so reset) when the offer changes. */
export function ConfirmStep({ name, busy, error, onFlash, onCancel, focusOnMount = false }: ConfirmStepProps) {
  const [c, setC] = useState<FlashConfirm>(EMPTY_CONFIRM);
  const firstBox = useRef<HTMLInputElement>(null);
  const nameId = useId();
  const helpId = useId();
  const missingId = useId();

  useEffect(() => {
    if (focusOnMount) firstBox.current?.focus();
  }, [focusOnMount]);

  const missing = confirmMissing(c, name);
  const valid = missing.length === 0;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!valid || busy) return;
    onFlash(c);
  };

  return (
    <form
      onSubmit={submit}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !busy) {
          e.stopPropagation();
          onCancel();
        }
      }}
      aria-label="Confirm flashing"
      className="flex flex-col gap-4 border-t border-border pt-4"
    >
      <fieldset className="flex flex-col">
        <legend className={cx(LABEL, "mb-1")}>Before you flash</legend>
        {FLASH_CHECKS.map(({ id, label }, i) => (
          <label key={id} className="flex min-h-11 cursor-pointer items-start gap-3 py-2 font-mono text-xs leading-relaxed text-foreground">
            <input
              ref={i === 0 ? firstBox : undefined}
              type="checkbox"
              checked={c.checks[id]}
              onChange={(e) => setC((prev) => ({ ...prev, checks: { ...prev.checks, [id]: e.target.checked } }))}
              className={CHECKBOX}
            />
            <span>{label}</span>
          </label>
        ))}
      </fieldset>

      <div className="flex flex-col gap-2">
        <label htmlFor={nameId} className={LABEL}>
          Type the device&apos;s name to confirm
        </label>
        <Input
          id={nameId}
          value={c.typed}
          maxLength={200}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          aria-describedby={helpId}
          onChange={(e) => setC((prev) => ({ ...prev, typed: e.target.value }))}
        />
        <p id={helpId} className={NOTE}>
          Type it exactly as shown, capitals included: <span className="break-words text-foreground">{name}</span>
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" variant="destructive" aria-disabled={!valid || undefined} aria-describedby={missingId} loading={busy}>
            Flash now
          </Button>
          <Button onClick={onCancel} aria-disabled={busy || undefined}>
            Cancel
          </Button>
        </div>
        <p id={missingId} className={NOTE}>
          {valid ? "" : missingText(missing)}
        </p>
        {/* Both lines stay in the page so a screen reader hears the text arrive. */}
        <p role="alert" className={error ? "max-w-prose font-mono text-xs leading-relaxed text-destructive" : "sr-only"}>
          {error ?? ""}
        </p>
      </div>
    </form>
  );
}

// ---------------------------------------------------------------- the panel

export interface FlashPanelViewProps {
  device: DeviceFacts;
  offer: FlashOffer;
  step: FlashStep;
  /** `actions` carries `flash`. Without it the opening button stays off. */
  canFlash?: boolean;
  busy?: boolean;
  error?: string | null;
  onOpen?: () => void;
  onCancel?: () => void;
  onFlash?: (c: FlashConfirm) => void;
  /** After a flash that has not been reported back: return to the offer. */
  onBack?: () => void;
  /** Slot beside the opening button: the hand-off to the agent. */
  agent?: ReactNode;
  focusOnMount?: boolean;
}

function Facts({ device, offer }: { device: DeviceFacts; offer: FlashOffer }) {
  const d = offerDisplay(device, offer);
  return (
    <>
      <div className="flex flex-col gap-2">
        <p className="font-mono text-sm leading-relaxed text-foreground">{FLASH_TITLE}</p>
        <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">
          <span className="text-foreground">{d.board}.</span> {d.why}
        </p>
        {d.check ? <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">{d.check}</p> : null}
      </div>

      <div className="flex flex-col gap-2">
        <SpecTable
          title="What would be written"
          rows={[
            // Paths and digests have no spaces to break at; everything else wraps at words.
            ...d.lines.map((l) => ({ label: l.label, value: l.label === "Image file" || l.label === "SHA-256" ? <span className="break-all">{l.value}</span> : l.value })),
            { label: "Comes from", value: d.source },
          ]}
        />
        <Disclosure summary="Show the full SHA-256">
          <div className="flex flex-col items-start gap-2">
            <code className="break-all border border-border bg-secondary/20 px-2 py-1 font-mono text-xs text-foreground">{d.sha.full}</code>
            <CopyButton text={d.sha.full} label="copy" ariaLabel="Copy the full SHA-256" />
          </div>
        </Disclosure>
      </div>

      <div className="border-2 border-foreground">
        <p className="bg-foreground px-3 py-1 font-mono text-[10px] uppercase tracking-widest text-background">Erases</p>
        <p className="p-3 font-mono text-sm leading-relaxed text-foreground">{d.erases}</p>
      </div>
    </>
  );
}

/**
 * The status block of the steps after "Flash now". It takes the focus when the element that had it just went away
 * (the button that was pressed, the form that vanished); it never pulls the focus away from where the user is.
 */
function Outcome({ children, focusOnMount }: { children: ReactNode; focusOnMount: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const active = typeof document === "undefined" ? null : document.activeElement;
    if (focusOnMount && (active === null || active === document.body)) ref.current?.focus();
  }, [focusOnMount]);
  return (
    <div ref={ref} tabIndex={-1} role="status" className="flex flex-col gap-2 focus-visible:outline-none">
      {children}
    </div>
  );
}

export function FlashPanelView({
  device,
  offer,
  step,
  canFlash = true,
  busy = false,
  error = null,
  onOpen,
  onCancel,
  onFlash,
  onBack,
  agent,
  focusOnMount = false,
}: FlashPanelViewProps) {
  const name = displayName(device);

  if (step === "gone") {
    return (
      <Panel title="firmware" heading={2} tint bodyClassName="flex flex-col gap-4">
        <Outcome focusOnMount={focusOnMount}>
          <p className="font-mono text-sm text-foreground">The node no longer offers firmware for this board.</p>
          <p className={NOTE}>
            {hasFirmwareNow(device) ? `It now reports: ${firmwareText(device)}.` : "Nothing was written from this page."} Nothing more to confirm here.
          </p>
          {onBack ? (
            <div>
              <Button size="sm" onClick={onBack}>
                Dismiss
              </Button>
            </div>
          ) : null}
        </Outcome>
      </Panel>
    );
  }

  // After "Flash now" the panel is only the outcome: the warning and the table belong to the decision.
  const deciding = step === "offer" || step === "confirm";
  const d = offerDisplay(device, offer);
  const written = `${d.lines[0]?.value ?? "firmware"} for ${d.lines[1]?.value ?? "this board"}`;

  return (
    <Panel title="firmware" heading={2} tint bodyClassName="flex flex-col gap-4">
      {deciding ? <Facts device={device} offer={offer} /> : null}

      {step === "offer" ? (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-start gap-3">
            <Button id={OPEN_ID} variant="primary" disabled={!canFlash} onClick={onOpen}>
              {flashButtonLabel(offer)}
            </Button>
            {agent}
          </div>
          <p className={NOTE}>
            {canFlash ? "Nothing is written until you confirm on the next step." : "This node does not allow flashing this board right now."}
          </p>
        </div>
      ) : null}

      {step === "confirm" ? (
        <ConfirmStep key={offer.image.sha256} name={name} busy={busy} error={error} onFlash={onFlash ?? (() => undefined)} onCancel={onCancel ?? (() => undefined)} focusOnMount={focusOnMount} />
      ) : null}

      {step === "started" ? (
        <Outcome focusOnMount={focusOnMount}>
          <p className="font-mono text-sm text-foreground">Flashing started: {written}.</p>
          <p className={NOTE}>Keep the board plugged in. This page updates when the board reports its new firmware.</p>
        </Outcome>
      ) : null}

      {step === "stalled" ? (
        <Outcome focusOnMount={false}>
          <p className="font-mono text-sm text-foreground">The board has not reported new firmware yet.</p>
          <p className={NOTE}>The node accepted the flash of {written} a while ago. If the board still shows no firmware, you can look at the offer again.</p>
          {onBack ? (
            <div>
              <Button size="sm" onClick={onBack}>
                Back to the offer
              </Button>
            </div>
          ) : null}
        </Outcome>
      ) : null}

      {step === "done" ? (
        <Outcome focusOnMount={focusOnMount}>
          <p className="font-mono text-sm text-foreground">{hasFirmwareNow(device) ? `Flashed: ${written}.` : "The offer is gone."}</p>
          <p className={NOTE}>
            {hasFirmwareNow(device)
              ? `The node now reports ${firmwareText(device)}. Nothing is bound to the board yet; the agent can set that up.`
              : "The node has not reported new firmware for this board yet."}
          </p>
        </Outcome>
      ) : null}
    </Panel>
  );
}

// ---------------------------------------------------------------- the container

type Phase = { kind: "offer" } | { kind: "confirm" } | { kind: "started"; at: number };

export interface FirmwareSectionProps {
  device: DeviceRecord;
  /** The hand-off to the agent, shown beside the opening button. */
  agent?: ReactNode;
}

/**
 * Renders nothing for a board without an offer. Keeps the last offer it saw, so the outcome of a flash (the offer
 * disappears when the board reports its new firmware) can still be told after the node has withdrawn it.
 */
export function FirmwareSection({ device, agent }: FirmwareSectionProps) {
  const offer = device.flash_offer ?? null;
  const kept = useRef<FlashOffer | null>(null);
  if (offer) kept.current = offer;
  const [phase, setPhase] = useState<Phase>({ kind: "offer" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const now = useNow(phase.kind === "started", 5_000);
  const lastKind = useRef(phase.kind);

  // Cancel (or "Back to the offer") hands the focus back to the button that opened the step.
  useEffect(() => {
    if (lastKind.current !== "offer" && phase.kind === "offer") document.getElementById(OPEN_ID)?.focus();
    lastKind.current = phase.kind;
  }, [phase.kind]);

  const shown = offer ?? kept.current;
  if (!shown) return null;

  let step: FlashStep;
  if (phase.kind === "started") step = !offer ? "done" : flashStalled(phase.at, now) ? "stalled" : "started";
  else if (!offer) {
    if (phase.kind === "offer") return null; // the node withdrew an offer nobody had opened: nothing to say
    step = "gone";
  } else step = phase.kind;

  const flash = async (c: FlashConfirm) => {
    if (!offer || busy || !confirmValid(c, displayName(device))) return;
    setBusy(true);
    setError(null);
    try {
      const adapter = await getAdapter();
      await adapter.flashDevice(device.id, flashRequest(offer, c));
      setPhase({ kind: "started", at: Date.now() });
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setError(flashErrorText(err));
      if (isApiError(err) && err.code === "offer_stale") void apiStore.refreshDevices();
    } finally {
      setBusy(false);
    }
  };

  const back = () => {
    setError(null);
    setPhase({ kind: "offer" });
  };

  return (
    <FlashPanelView
      device={device}
      offer={shown}
      step={step}
      canFlash={device.actions.includes("flash")}
      busy={busy}
      error={error}
      agent={agent}
      focusOnMount
      onOpen={() => setPhase({ kind: "confirm" })}
      onCancel={back}
      onFlash={(c) => void flash(c)}
      onBack={back}
    />
  );
}
