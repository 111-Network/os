// Maps the pure FactRow values from model.ts onto SpecTable rows (stacked lines, copy buttons).
import { CopyButton } from "../../components/copy-button";
import type { SpecRow } from "../../components/spec-table";
import type { FactRow } from "./model";

function Value({ row }: { row: FactRow }) {
  const body = Array.isArray(row.value) ? (
    <span className="flex flex-col items-end gap-0.5">
      {row.value.map((line, i) => (
        <span key={i} className="break-all">
          {line}
        </span>
      ))}
    </span>
  ) : (
    <span className="break-all">{row.value}</span>
  );
  if (row.copy === undefined) return body;
  return (
    <span className="flex flex-wrap items-center justify-end gap-2">
      {body}
      <CopyButton text={row.copy} ariaLabel={`Copy ${row.label}`} />
    </span>
  );
}

export function toSpecRows(rows: FactRow[]): SpecRow[] {
  return rows.map((row) => ({ label: row.label, value: <Value row={row} /> }));
}
