import { cx } from "../lib/cx";

const GLYPHS = "▁▂▃▄▅▆▇█";

/**
 * Map numbers to block glyphs. Scales between `min` and `max` (defaults: the data's own range).
 * A flat series renders mid-height; non-finite values render as a middle dot.
 */
export function sparkGlyphs(values: number[], min?: number, max?: number): string {
  const finite = values.filter((v) => Number.isFinite(v));
  if (finite.length === 0) return values.map(() => "·").join("");
  const lo = min ?? Math.min(...finite);
  const hi = max ?? Math.max(...finite);
  const span = hi - lo;
  return values
    .map((v) => {
      if (!Number.isFinite(v)) return "·";
      if (span <= 0) return GLYPHS.charAt(3);
      const t = Math.min(1, Math.max(0, (v - lo) / span));
      return GLYPHS.charAt(Math.round(t * (GLYPHS.length - 1)));
    })
    .join("");
}

export interface SparklineProps {
  values: number[];
  /** Required: what the chart says, in words (the glyphs themselves are hidden from AT). */
  label: string;
  min?: number;
  max?: number;
  className?: string;
}

export function Sparkline({ values, label, min, max, className }: SparklineProps) {
  return (
    <span role="img" aria-label={label} className={cx("inline-block font-mono leading-none tracking-normal text-foreground", className)}>
      <span aria-hidden="true">{values.length === 0 ? "—" : sparkGlyphs(values, min, max)}</span>
    </span>
  );
}
