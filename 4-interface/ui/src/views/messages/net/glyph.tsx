// The delivery-evidence glyph for a foreign row (os/6-transport/README.md §4): three pips after the outbox, filled as far as the
// evidence got, and a crossed box for a refusal before air. Shape and the word carry the state, never color.
import { cx } from "../../../lib/cx";
import { foreignStateCopy } from "./model";

const PIPS: Record<string, { filled: number; label: string }> = {
  queued: { filled: 0, label: "queued" },
  sent: { filled: 1, label: "sent" },
  relayed: { filled: 2, label: "relayed" },
  delivered: { filled: 3, label: "delivered" },
};

export function ForeignGlyph({ state, className, label }: { state: string; className?: string | undefined; label?: string | undefined }) {
  const known = Object.prototype.hasOwnProperty.call(PIPS, state) ? PIPS[state]! : null;
  const failed = state === "failed";
  const pips = known ? "■".repeat(known.filled) + "□".repeat(3 - known.filled) : failed ? "[x]" : "???";
  return (
    <span className={cx("inline-flex items-center gap-1.5 whitespace-nowrap font-mono text-[11px] text-muted-foreground", className)} title={foreignStateCopy(state)}>
      <span aria-hidden="true" className="tracking-tighter">
        {pips}
      </span>
      <span className="text-foreground">{label ?? (known ? known.label : failed ? "failed" : "unknown")}</span>
    </span>
  );
}

/** The network chip every foreign message carries: the manifest's short mark, as text. */
export function NetChip({ chip, className }: { chip: string; className?: string }) {
  return (
    <span className={cx("inline-block border border-border px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-widest text-muted-foreground", className)} data-testid="net-chip">
      {chip}
    </span>
  );
}
