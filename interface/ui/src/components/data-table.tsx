import { useState, type ReactNode } from "react";
import { cx, FOCUS_RING } from "../lib/cx";

export interface DataColumn<T> {
  key: string;
  header: string;
  cell: (row: T) => ReactNode;
  align?: "start" | "end";
  /** Render the cell in muted-foreground instead of foreground. */
  muted?: boolean;
  className?: string;
}

export interface DataTableProps<T> {
  /** Required: names the table for assistive tech (visually hidden). */
  caption: string;
  columns: DataColumn<T>[];
  rows: T[];
  rowKey: (row: T, index: number) => string;
  /** Return content to make a row expandable; return null/undefined for rows with nothing to show. */
  expand?: (row: T) => ReactNode;
  /** Shown in place of rows when `rows` is empty. */
  empty?: ReactNode;
  className?: string;
}

/**
 * Real <table> semantics. Row expansion is a button (`aria-expanded`, `aria-controls`) plus a following
 * row, because <details> is not valid inside table markup.
 */
export function DataTable<T>({ caption, columns, rows, rowKey, expand, empty, className }: DataTableProps<T>) {
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const span = columns.length + (expand ? 1 : 0);

  const toggle = (key: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  return (
    <div className={cx("overflow-x-auto border border-border", className)}>
      <table className="w-full border-collapse divide-y divide-border font-mono text-xs">
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr>
            {expand ? (
              <th scope="col" className="w-11 px-0 py-2">
                <span className="sr-only">Details</span>
              </th>
            ) : null}
            {columns.map((c) => (
              <th
                key={c.key}
                scope="col"
                className={cx(
                  "px-4 py-2 text-[10px] font-normal uppercase tracking-widest text-muted-foreground",
                  c.align === "end" ? "text-end" : "text-start",
                )}
              >
                {c.header}
              </th>
            ))}
          </tr>
        </thead>
        {rows.length === 0 ? (
          <tbody>
            <tr>
              <td colSpan={span} className="px-4 py-6 text-muted-foreground">
                {empty ?? "Nothing to show."}
              </td>
            </tr>
          </tbody>
        ) : (
          rows.map((row, i) => {
            const key = rowKey(row, i);
            const extra = expand ? expand(row) : null;
            const expandable = extra !== null && extra !== undefined && extra !== false;
            const isOpen = expandable && open.has(key);
            const panelId = `dt-${key}`.replace(/\s+/g, "-");
            return (
              <tbody key={key}>
                <tr className="transition-colors duration-200 hover:bg-secondary/50">
                  {expand ? (
                    <td className="w-11 p-0 align-top">
                      {expandable ? (
                        <button
                          type="button"
                          aria-expanded={isOpen}
                          aria-controls={panelId}
                          aria-label={isOpen ? "Hide details" : "Show details"}
                          onClick={() => toggle(key)}
                          className={cx(
                            "flex min-h-11 w-11 items-start justify-center pt-2.5 font-mono text-muted-foreground hover:text-foreground",
                            FOCUS_RING,
                          )}
                        >
                          <span aria-hidden="true">{isOpen ? "v" : ">"}</span>
                        </button>
                      ) : null}
                    </td>
                  ) : null}
                  {columns.map((c) => (
                    <td
                      key={c.key}
                      className={cx(
                        "px-4 py-2.5 align-top",
                        c.align === "end" ? "text-end" : "text-start",
                        c.muted ? "text-muted-foreground" : "text-foreground",
                        c.className,
                      )}
                    >
                      {c.cell(row)}
                    </td>
                  ))}
                </tr>
                {expandable ? (
                  <tr id={panelId} hidden={!isOpen}>
                    <td colSpan={span} className="bg-secondary/20 px-4 py-3 text-muted-foreground">
                      {extra}
                    </td>
                  </tr>
                ) : null}
              </tbody>
            );
          })
        )}
      </table>
    </div>
  );
}
