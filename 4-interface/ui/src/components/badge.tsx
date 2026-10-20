import type { HTMLAttributes } from "react";
import { cx } from "../lib/cx";

export type BadgeProps = HTMLAttributes<HTMLSpanElement>;

export function Badge({ className, ...rest }: BadgeProps) {
  return <span {...rest} className={cx("border border-border px-2 py-1 font-mono text-[10px] text-muted-foreground", className)} />;
}
