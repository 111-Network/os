// The "NEEDS FIRMWARE" text tag beside a board's NOT SET dot (device list, Overview). Text only: no new colour.
import { cx } from "../../lib/cx";
import { NEEDS_FIRMWARE } from "./flash-model";

export function NeedsFirmwareTag({ className }: { className?: string }) {
  return <span className={cx("shrink-0 font-mono text-[10px] uppercase tracking-widest text-foreground", className)}>{NEEDS_FIRMWARE}</span>;
}
