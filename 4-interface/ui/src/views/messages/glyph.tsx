// The delivery-evidence glyph (os/50-messaging/README.md section 9): three pips after the outbox, filled as far as the
// evidence got. Shape and the word carry the state, never color; the title and the detail disclosure hold the exact copy.
import { cx } from "../../lib/cx";
import { glyphFor, stateCopy } from "./model";

/**
 * `copy` and `label` replace the state's own sentence and word for the one case the state alone does not tell: an own message
 * that was cancelled is `queued` on the node, but "in the outbox" would be false of it (§4: an empty tombstone goes instead).
 */
export function StateGlyph({ state, className, copy, label }: { state: string; className?: string; copy?: string | undefined; label?: string | undefined }) {
  const g = glyphFor(state);
  const pips = g.known ? "■".repeat(g.filled) + "□".repeat(3 - g.filled) : "???";
  return (
    <span className={cx("inline-flex items-center gap-1.5 whitespace-nowrap font-mono text-[11px] text-muted-foreground", className)} title={copy ?? stateCopy(state)}>
      <span aria-hidden="true" className="tracking-tighter">
        {pips}
      </span>
      <span className="text-foreground">{label ?? g.label}</span>
    </span>
  );
}
