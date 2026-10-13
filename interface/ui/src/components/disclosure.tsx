import { useState, type DetailsHTMLAttributes, type ReactNode } from "react";
import { cx, FOCUS_RING } from "../lib/cx";

export interface DisclosureProps extends Omit<DetailsHTMLAttributes<HTMLDetailsElement>, "open" | "summary"> {
  summary: ReactNode;
  defaultOpen?: boolean;
  children?: ReactNode;
}

/** Native <details>/<summary>. `>` closed, `v` open, no animation. Keyboard and AT behaviour come from the platform. */
export function Disclosure({ summary, defaultOpen = false, className, children, onToggle, ...rest }: DisclosureProps) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <details
      {...rest}
      open={defaultOpen}
      onToggle={(e) => {
        setOpen(e.currentTarget.open);
        onToggle?.(e);
      }}
      className={className}
    >
      <summary
        className={cx(
          "flex min-h-11 cursor-pointer list-none items-center gap-2 font-mono text-xs text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden",
          FOCUS_RING,
        )}
      >
        <span aria-hidden="true" className="inline-block w-3 shrink-0 text-center">
          {open ? "v" : ">"}
        </span>
        <span className="min-w-0">{summary}</span>
      </summary>
      <div className="pb-2 ps-5 pt-1">{children}</div>
    </details>
  );
}
