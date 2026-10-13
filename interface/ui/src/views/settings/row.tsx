// Layout shared by the Essentials and Sharing panels: a label with help text on the start side and
// the control on the end side (stacked below `md`). Every row is a named group (not a landmark) for assistive tech.
import type { ReactNode } from "react";

export const FEEDBACK = "font-mono text-xs leading-relaxed";

export interface RowProps {
  id: string;
  label: string;
  help?: ReactNode;
  children: ReactNode;
}

export function Row({ id, label, help, children }: RowProps) {
  return (
    <div
      data-row={id}
      role="group"
      aria-labelledby={`${id}-label`}
      className="grid gap-3 border-b border-border p-4 last:border-b-0 md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] md:gap-8"
    >
      <div className="min-w-0">
        <h2 id={`${id}-label`} className="font-mono text-[10px] uppercase tracking-widest text-foreground">
          {label}
        </h2>
        {help ? <p className={`mt-2 max-w-prose text-muted-foreground ${FEEDBACK}`}>{help}</p> : null}
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** One inline line of feedback under a control: polite status, or an assertive problem. */
export function Feedback({ text, problem = false }: { text: string | null; problem?: boolean }) {
  // One element whether or not there is text, so a screen reader hears the text arrive in a live region it already knows.
  return (
    <p role={problem ? "alert" : "status"} className={text ? `mt-2 ${FEEDBACK} ${problem ? "text-destructive" : "text-muted-foreground"}` : "sr-only"}>
      {text ? (problem ? `Problem: ${text}` : text) : ""}
    </p>
  );
}
