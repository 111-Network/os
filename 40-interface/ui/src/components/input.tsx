import type { ComponentPropsWithRef } from "react";
import { cx, FOCUS_RING } from "../lib/cx";

/**
 * Shared field look. `text-base md:text-sm`: below the md breakpoint the font stays at 16 px so iOS Safari
 * does not zoom the page on focus (a plain `text-sm` would).
 */
export const FIELD_CLASS = cx(
  "w-full border border-input bg-background px-3 py-2 font-mono text-base text-foreground md:text-sm",
  "placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive",
  FOCUS_RING,
);

export type InputProps = ComponentPropsWithRef<"input">;

export function Input({ className, type = "text", ...rest }: InputProps) {
  return <input {...rest} type={type} className={cx(FIELD_CLASS, "min-h-11", className)} />;
}
