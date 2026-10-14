// The delivery-evidence glyph (os/messaging/README.md section 9): three pips after the outbox, filled as far as the
// evidence got. Shape and the word carry the state, never color; the title and the detail disclosure hold the exact copy.
import { cx } from "../../lib/cx";
import { glyphFor, stateCopy } from "./model";

export function StateGlyph({ state, className }: { state: string; className?: string }) {
  const g = glyphFor(state);
  const pips = g.known ? "■".repeat(g.filled) + "□".repeat(3 - g.filled) : "???";
  return (
    <span className={cx("inline-flex items-center gap-1.5 whitespace-nowrap font-mono text-[11px] text-muted-foreground", className)} title={stateCopy(state)}>
      <span aria-hidden="true" className="tracking-tighter">
        {pips}
      </span>
      <span className="text-foreground">{g.label}</span>
    </span>
  );
}
