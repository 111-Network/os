import { cx } from "../lib/cx";

/** Screen-reader text for the count (pure, unit-testable). */
export function attentionLabel(count: number): string {
  return count === 1 ? "1 item needs you" : `${count} items need you`;
}

export interface AttentionBadgeProps {
  count: number;
  className?: string;
}

/** Count pill, inverted. Renders nothing at 0. */
export function AttentionBadge({ count, className }: AttentionBadgeProps) {
  if (!Number.isFinite(count) || count <= 0) return null;
  const n = Math.floor(count);
  return (
    <span
      role="img"
      aria-label={attentionLabel(n)}
      className={cx(
        "inline-flex min-w-5 items-center justify-center border border-foreground bg-foreground px-1.5 py-0.5 font-mono text-[10px] leading-none text-background",
        className,
      )}
    >
      <span aria-hidden="true">{n > 99 ? "99+" : n}</span>
    </span>
  );
}
