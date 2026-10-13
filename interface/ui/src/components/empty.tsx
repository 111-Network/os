import type { ReactNode } from "react";
import { cx } from "../lib/cx";

export interface EmptyProps {
  /** What is missing or degraded, in plain words. */
  title: string;
  /** Small uppercase label above the title, e.g. "DEGRADED". */
  kicker?: string;
  /** One honest sentence: why, and what happens next. */
  children?: ReactNode;
  /** Optional action slot (a Button or ButtonLink). */
  action?: ReactNode;
  className?: string;
}

/** Honest empty / degraded block. No fake data, no spinner theatre. */
export function Empty({ title, kicker, children, action, className }: EmptyProps) {
  return (
    <div className={cx("border border-dashed border-border p-6", className)}>
      {kicker ? <p className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{kicker}</p> : null}
      <p className={cx("font-mono text-sm text-foreground", kicker && "mt-1")}>{title}</p>
      {children ? <p className="mt-2 max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">{children}</p> : null}
      {action ? <div className="mt-4">{action}</div> : null}
    </div>
  );
}
