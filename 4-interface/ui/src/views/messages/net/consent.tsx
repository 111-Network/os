// The consent round-trip in the UI (os/60-transport/README.md §2.1, §8; interface README 7.x): a 409 `consent_required` carries the
// exact text and its SHA-256. This panel shows that text verbatim (as text only: it may quote an untrusted channel name), badges the
// live add-ons, and its Confirm button repeats the call with the hash. A text that changes between the two calls has a new hash and
// is shown again, never confirmed on the old reading. Inline, not a modal (DESIGN.md): focus moves to it, Escape cancels.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Badge } from "../../../components/badge";
import { Button } from "../../../components/button";
import { consentOf } from "../../../lib/api/bridge";
import type { ConsentRequired } from "../../../lib/api/types";

/** What each add-on the node names means in a badge. An unknown add-on shows as it came, cleaned, as text. */
const ADDON_WORD: Record<string, string> = {
  default_key_source: "default key: anyone can read the source",
  uplinked_destination: "destination is published to the internet",
  licensed_destination: "licensed account",
  licensed_account: "licensed account",
  uplinked_channel: "published to the internet",
};

const ACTION_WORD: Record<ConsentRequired["action"], string> = {
  edge_enable: "Start this relay",
  edge_resume: "Resume this relay",
  component_enable: "Install and turn this on",
  ham_confirm: "Confirm readable sends from a licensed board",
  uplink_send: "Turn on sending on an uplinked channel",
};

export const addonWord = (a: string): string => (Object.prototype.hasOwnProperty.call(ADDON_WORD, a) ? ADDON_WORD[a]! : a.replace(/[^\w .:/-]/g, "").slice(0, 60));
export const actionWord = (a: ConsentRequired["action"]): string => (Object.prototype.hasOwnProperty.call(ACTION_WORD, a) ? ACTION_WORD[a] : "Confirm");

export interface ConsentPanelProps {
  consent: ConsentRequired;
  /** The text changed since the person last read it (a live fact moved): say so. */
  changed?: boolean;
  busy?: boolean;
  /** Confirm the text on screen; the caller echoes that text's hash (`runConsent` does). */
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
}

export function ConsentPanel({ consent, changed = false, busy = false, onConfirm, onCancel, confirmLabel }: ConsentPanelProps) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.focus();
  }, [consent.hash]);
  return (
    <div
      ref={ref}
      role="alertdialog"
      aria-modal="false"
      aria-label={`Confirm: ${actionWord(consent.action)}`}
      aria-describedby="consent-text"
      tabIndex={-1}
      data-testid="consent-panel"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onCancel();
        }
      }}
      className="flex flex-col gap-3 border border-foreground p-4 font-mono"
    >
      <p className="text-[10px] uppercase tracking-widest text-muted-foreground">confirm · {actionWord(consent.action)}</p>
      {changed ? (
        <p role="status" className="text-xs leading-relaxed text-foreground" data-testid="consent-changed">
          The text changed since you last read it, so it needs a new confirmation. Read it again.
        </p>
      ) : null}
      <pre id="consent-text" className="whitespace-pre-wrap break-words border border-border bg-secondary/20 p-3 text-sm leading-relaxed text-foreground" data-testid="consent-text">
        {consent.warning_text}
      </pre>
      {consent.addons.length > 0 ? (
        <ul className="flex flex-wrap gap-2" aria-label="What makes this text stronger">
          {consent.addons.map((a) => (
            <li key={a}>
              <Badge className="border-foreground text-foreground">{addonWord(a)}</Badge>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="primary" arrow={false} loading={busy} onClick={onConfirm} data-testid="consent-confirm">
          {confirmLabel ?? "I have read this: confirm"}
        </Button>
        <Button size="sm" onClick={onCancel} data-testid="consent-cancel">
          Cancel
        </Button>
      </div>
    </div>
  );
}

/** How a consented call ended: it went through, the person said no, or it failed for another reason. */
export type ConsentOutcome<T> = { ok: true; value: T } | { ok: false; cancelled: true } | { ok: false; cancelled: false; error: unknown };

/**
 * The round-trip without a screen: `call(undefined)` first; on a 409 `consent_required` it asks the person (`ask` resolves true to confirm,
 * false to cancel) and repeats `call(consent.hash, consent)`, echoing exactly the hash of the text that was asked about. A second 409
 * with a different text asks again with `changed` set; the same hash twice, or any other failure, ends it with the error. With `initial`
 * the person is asked at once, before any call (a fan-out preview already named the text).
 */
export async function runConsent<T>(
  call: (hash?: string, consent?: ConsentRequired) => Promise<T>,
  ask: (consent: ConsentRequired, changed: boolean) => Promise<boolean>,
  initial?: ConsentRequired,
): Promise<ConsentOutcome<T>> {
  let hash: string | undefined;
  let shown: ConsentRequired | undefined;
  let changed = false;
  if (initial) {
    if (!(await ask(initial, false))) return { ok: false, cancelled: true };
    hash = initial.hash;
    shown = initial;
    changed = true;
  }
  for (;;) {
    try {
      return { ok: true, value: await call(hash, shown) };
    } catch (error) {
      const consent = consentOf(error);
      // The same hash twice would loop on a daemon that never accepts it; the second time it is a plain failure.
      if (!consent || (hash !== undefined && consent.hash === hash)) return { ok: false, cancelled: false, error };
      if (!(await ask(consent, changed))) return { ok: false, cancelled: true };
      hash = consent.hash;
      shown = consent;
      changed = true;
    }
  }
}

interface Open {
  consent: ConsentRequired;
  changed: boolean;
  busy: boolean;
  confirm: () => void;
  cancel: () => void;
}

/**
 * `attempt(call)` is `runConsent` with the panel as the question. Render `dialog` where the panel belongs. `call(hash, consent)`
 * gets the text being confirmed, so it can route the confirmation (a HAM switch, an uplink switch) before it repeats the act.
 */
export function useConsentFlow(): {
  dialog: ReactNode;
  attempt: <T>(call: (hash?: string, consent?: ConsentRequired) => Promise<T>, initial?: ConsentRequired) => Promise<ConsentOutcome<T>>;
  pending: boolean;
} {
  const [open, setOpen] = useState<Open | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const attempt = useCallback(
    <T,>(call: (hash?: string, consent?: ConsentRequired) => Promise<T>, initial?: ConsentRequired) =>
      runConsent(
        call,
        (consent, changed) =>
          new Promise<boolean>((resolve) => {
            if (!alive.current) return resolve(false);
            setOpen({
              consent,
              changed,
              busy: false,
              confirm: () => {
                setOpen((o) => (o ? { ...o, busy: true } : o));
                resolve(true);
              },
              cancel: () => {
                setOpen(null);
                resolve(false);
              },
            });
          }),
        initial,
      ).then((out) => {
        if (alive.current) setOpen(null);
        return out;
      }),
    [],
  );
  const dialog = open ? <ConsentPanel consent={open.consent} changed={open.changed} busy={open.busy} onConfirm={open.confirm} onCancel={open.cancel} /> : null;
  return { dialog, attempt, pending: open !== null };
}
