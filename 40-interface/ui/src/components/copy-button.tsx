import { useEffect, useRef, useState } from "react";
import { cx } from "../lib/cx";
import { buttonClasses, type ButtonVariant } from "./button";
import { FIELD_CLASS } from "./input";

/** Last-resort synchronous copy through a hidden textarea. Must run inside the click handler's stack. */
function legacyCopy(text: string): boolean {
  if (typeof document === "undefined" || typeof document.execCommand !== "function") return false;
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.setAttribute("aria-hidden", "true");
  ta.style.position = "fixed";
  ta.style.insetBlockStart = "0";
  ta.style.insetInlineStart = "0";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  let ok = false;
  try {
    ta.focus();
    ta.select();
    ta.setSelectionRange(0, text.length);
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  } finally {
    document.body.removeChild(ta);
    previous?.focus();
  }
  return ok;
}

/**
 * Copy text: `navigator.clipboard` when available (secure contexts only; plain-HTTP LAN origins are not),
 * then the hidden-textarea `execCommand` fallback. Resolves false when both fail.
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // permission denied or document not focused: fall through
    }
  }
  return legacyCopy(text);
}

export interface CopyButtonProps {
  text: string;
  /** Idle label, default "copy". */
  label?: string;
  /** Accessible name for the idle button when the label alone is ambiguous, e.g. "Copy node id". */
  ariaLabel?: string;
  variant?: ButtonVariant;
  className?: string;
}

type CopyState = "idle" | "copied" | "manual";

export function CopyButton({ text, label = "copy", ariaLabel, variant = "default", className }: CopyButtonProps) {
  const [state, setState] = useState<CopyState>("idle");
  const timer = useRef<number | undefined>(undefined);
  const area = useRef<HTMLTextAreaElement>(null);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  useEffect(() => {
    if (state === "manual") {
      area.current?.focus();
      area.current?.select();
    }
  }, [state]);

  const onClick = async () => {
    window.clearTimeout(timer.current);
    if (await copyToClipboard(text)) {
      setState("copied");
      timer.current = window.setTimeout(() => setState("idle"), 1500);
    } else {
      setState("manual");
    }
  };

  return (
    <span className={cx("inline-flex flex-col items-start gap-2", className)}>
      <button type="button" aria-label={ariaLabel} onClick={() => void onClick()} className={buttonClasses({ variant, size: "sm" })}>
        {state === "copied" ? "copied" : label}
      </button>
      <span role="status" className="sr-only">
        {state === "copied" ? "copied to clipboard" : ""}
      </span>
      {state === "manual" ? (
        <span className="flex w-full min-w-64 flex-col gap-1">
          <textarea
            ref={area}
            readOnly
            value={text}
            rows={Math.min(8, Math.max(2, text.split("\n").length))}
            aria-label="Select to copy"
            onKeyDown={(e) => {
              if (e.key === "Escape") setState("idle");
            }}
            className={cx(FIELD_CLASS, "resize-y text-xs leading-relaxed")}
          />
          <span className="flex items-center gap-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            select to copy
            <button type="button" onClick={() => setState("idle")} className="min-h-11 px-2 hover:text-foreground focus-visible:ring-2 focus-visible:ring-foreground focus-visible:outline-none">
              [done]
            </button>
          </span>
        </span>
      ) : null}
    </span>
  );
}
