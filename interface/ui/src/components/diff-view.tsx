import { cx } from "../lib/cx";

export type DiffLineKind = "add" | "del" | "hunk" | "meta" | "ctx";

/** Classify one line of a unified diff (pure, unit-testable). */
export function classifyDiffLine(line: string): DiffLineKind {
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff ") || line.startsWith("index ")) return "meta";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "ctx";
}

const KIND_CLASS: Record<DiffLineKind, string> = {
  add: "text-foreground",
  del: "text-muted-foreground",
  hunk: "text-muted-foreground",
  meta: "text-muted-foreground",
  ctx: "text-muted-foreground",
};

export interface DiffViewProps {
  /** Unified diff text. */
  diff: string;
  /** Accessible name, default "diff". */
  label?: string;
  className?: string;
}

/** Monochrome unified-diff viewer. The +/- characters stay in the text so it reads without any styling. */
export function DiffView({ diff, label = "diff", className }: DiffViewProps) {
  const lines = diff.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n");
  return (
    <pre
      role="group"
      aria-label={label}
      tabIndex={0}
      className={cx(
        "overflow-x-auto border border-border bg-secondary/20 p-3 font-mono text-xs leading-relaxed focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none",
        className,
      )}
    >
      {lines.map((line, i) => (
        <span key={i} className={cx("block", KIND_CLASS[classifyDiffLine(line)], classifyDiffLine(line) === "hunk" && "mt-1 first:mt-0")}>
          {line === "" ? " " : line}
        </span>
      ))}
    </pre>
  );
}
