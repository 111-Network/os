import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { cx, FOCUS_RING } from "../lib/cx";

export interface SegmentedOption<T extends string> {
  value: T;
  label: ReactNode;
  disabled?: boolean;
}

export interface SegmentedProps<T extends string> {
  value: T;
  onChange: (next: T) => void;
  options: SegmentedOption<T>[];
  /** Accessible name of the group, e.g. "Interface mode". */
  label: string;
  className?: string;
}

/** Index to move to from `from` in direction `dir`, skipping disabled options, wrapping. */
export function nextEnabled(disabled: boolean[], from: number, dir: 1 | -1): number {
  const n = disabled.length;
  let i = from;
  for (let s = 0; s < n; s++) {
    i = (i + dir + n) % n;
    if (!disabled[i]) return i;
  }
  return from;
}

/**
 * Single-select radio group (e.g. SIMPLE | ADVANCED). Roving tabindex; arrows move and select
 * (logical: ArrowRight is "next" in LTR, "previous" in RTL); Home/End jump.
 */
export function Segmented<T extends string>({ value, onChange, options, label, className }: SegmentedProps<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const selected = options.findIndex((o) => o.value === value);
  const firstEnabled = options.findIndex((o) => !o.disabled);
  const tabStop = selected >= 0 ? selected : firstEnabled;

  const go = (index: number) => {
    const opt = options[index];
    if (!opt || opt.disabled) return;
    onChange(opt.value);
    refs.current[index]?.focus();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const rtl = getComputedStyle(e.currentTarget).direction === "rtl";
    const dis = options.map((o) => !!o.disabled);
    let target = -1;
    switch (e.key) {
      case "ArrowRight":
        target = nextEnabled(dis, index, rtl ? -1 : 1);
        break;
      case "ArrowDown":
        target = nextEnabled(dis, index, 1);
        break;
      case "ArrowLeft":
        target = nextEnabled(dis, index, rtl ? 1 : -1);
        break;
      case "ArrowUp":
        target = nextEnabled(dis, index, -1);
        break;
      case "Home":
        target = dis.findIndex((d) => !d);
        break;
      case "End":
        target = dis.lastIndexOf(false);
        break;
      default:
        return;
    }
    e.preventDefault();
    if (target >= 0) go(target);
  };

  return (
    <div role="radiogroup" aria-label={label} className={cx("inline-flex", className)}>
      {options.map((o, i) => {
        const on = i === selected;
        return (
          <button
            key={o.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={on}
            disabled={o.disabled}
            tabIndex={i === tabStop ? 0 : -1}
            onClick={() => go(i)}
            onKeyDown={(e) => onKeyDown(e, i)}
            className={cx(
              "relative -ms-px min-h-11 border px-4 py-2 font-mono text-xs transition-all duration-200 first:ms-0 hover:z-10 focus-visible:z-10 disabled:pointer-events-none disabled:opacity-50",
              FOCUS_RING,
              on
                ? "z-10 border-foreground bg-foreground text-background"
                : "border-border text-muted-foreground hover:border-foreground hover:text-foreground",
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
