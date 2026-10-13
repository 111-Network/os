import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx, FOCUS_RING } from "../lib/cx";

export interface ToggleProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange" | "children"> {
  pressed: boolean;
  onPressedChange: (next: boolean) => void;
  /** An action is in flight: clicks are ignored but the toggle stays focusable (a `disabled` button loses focus). */
  busy?: boolean;
  children: ReactNode;
}

/** The DESIGN.md interactive toggle: solid inversion when on. 44 px minimum height. */
export function Toggle({ pressed, onPressedChange, busy = false, className, children, onClick, ...rest }: ToggleProps) {
  return (
    <button
      type="button"
      {...rest}
      aria-pressed={pressed}
      aria-disabled={busy || undefined}
      aria-busy={busy || undefined}
      onClick={(e) => {
        if (busy) return;
        onClick?.(e);
        if (!e.defaultPrevented) onPressedChange(!pressed);
      }}
      className={cx(
        "min-h-11 border p-3 text-start font-mono transition-all duration-200 disabled:pointer-events-none disabled:opacity-50 aria-disabled:cursor-default aria-disabled:opacity-50",
        FOCUS_RING,
        pressed
          ? "border-foreground bg-foreground text-background"
          : "border-border text-muted-foreground hover:border-foreground hover:text-foreground",
        className,
      )}
    >
      {children}
    </button>
  );
}
