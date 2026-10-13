import type { ReactNode } from "react";
import { cx } from "../lib/cx";

export interface SectionHeaderProps {
  kicker: string;
  title: string;
  /** Heading level, default 2. A view's own title is level 1. */
  level?: 1 | 2 | 3;
  /** Optional slot at the end (status, small actions). */
  end?: ReactNode;
  className?: string;
}

export function SectionHeader({ kicker, title, level = 2, end, className }: SectionHeaderProps) {
  const Heading = `h${level}` as const;
  return (
    <header className={cx("flex items-end justify-between gap-4", className)}>
      <div className="min-w-0">
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{kicker}</span>
        <Heading className="mt-1 font-pixel-line text-2xl font-bold text-foreground md:text-3xl">{title}</Heading>
      </div>
      {end ? <div className="shrink-0">{end}</div> : null}
    </header>
  );
}
