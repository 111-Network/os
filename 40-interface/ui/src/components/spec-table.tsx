import type { ReactNode } from "react";
import { cx } from "../lib/cx";

/** Density mode (spec section 5). Structurally identical to the app's own mode type. */
export type UiMode = "simple" | "advanced";

export interface SpecRow {
  label: string;
  value: ReactNode;
  /** Lowest mode that shows this row. Absent or "simple" = always; "advanced" = advanced mode only. */
  minMode?: UiMode;
}

export interface SpecTableProps {
  rows: SpecRow[];
  /** Current mode. Omit to show every row. */
  mode?: UiMode;
  /** Optional small header (square + uppercase label), as in DESIGN.md. */
  title?: string;
  /** Drop the outer border (when nested in a flush Panel). */
  bare?: boolean;
  className?: string;
}

/** Rows visible in `mode` (pure, unit-testable). */
export function visibleSpecRows(rows: SpecRow[], mode: UiMode = "advanced"): SpecRow[] {
  return rows.filter((r) => r.minMode === undefined || r.minMode === "simple" || mode === "advanced");
}

export function SpecTable({ rows, mode, title, bare = false, className }: SpecTableProps) {
  const shown = visibleSpecRows(rows, mode);
  return (
    <div className={cx(!bare && "border border-border", className)}>
      {title ? (
        <div className="flex items-center gap-2 border-b border-border px-4 py-2">
          <div aria-hidden="true" className="h-1.5 w-1.5 bg-foreground" />
          <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{title}</span>
        </div>
      ) : null}
      <dl className="divide-y divide-border">
        {shown.map((row) => (
          <div
            key={row.label}
            className="flex items-start justify-between gap-4 px-4 py-2.5 font-mono text-xs transition-colors duration-200 hover:bg-secondary/50"
          >
            <dt className="shrink-0 text-muted-foreground">{row.label}</dt>
            <dd className="min-w-0 break-words text-end text-foreground">{row.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
