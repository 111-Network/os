import { cx } from "../lib/cx";
import { Disclosure } from "./disclosure";

/** Pretty-print any value; never throws (circular structures and BigInt fall back to String()). */
export function prettyJson(value: unknown): string {
  try {
    const out = JSON.stringify(value, null, 2);
    return out === undefined ? String(value) : out;
  } catch {
    return String(value);
  }
}

export interface JsonBlockProps {
  value: unknown;
  /** Disclosure summary, default "JSON". */
  summary?: string;
  defaultOpen?: boolean;
  className?: string;
}

export function JsonBlock({ value, summary = "JSON", defaultOpen = false, className }: JsonBlockProps) {
  return (
    <Disclosure summary={summary} defaultOpen={defaultOpen} className={className}>
      <pre
        tabIndex={0}
        className={cx(
          "max-h-96 overflow-auto border border-border bg-secondary/20 p-3 font-mono text-xs leading-relaxed text-muted-foreground",
          "focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground focus-visible:outline-none",
        )}
      >
        {prettyJson(value)}
      </pre>
    </Disclosure>
  );
}
