import type { ComponentPropsWithRef } from "react";
import { cx } from "../lib/cx";
import { FIELD_CLASS } from "./input";

export type TextareaProps = ComponentPropsWithRef<"textarea">;

export function Textarea({ className, rows = 3, ...rest }: TextareaProps) {
  return <textarea {...rest} rows={rows} className={cx(FIELD_CLASS, "resize-y leading-relaxed", className)} />;
}
