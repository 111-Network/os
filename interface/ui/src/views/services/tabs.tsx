// A small tab list for the service page ("Details" | "Panel"). Roving tabindex: arrows, Home and End move and select
// (ArrowRight is "next" in LTR and "previous" in RTL, like Segmented). The panels stay in the page and are hidden when not
// selected, so a framed panel is not torn down when the person looks at the details and comes back.
import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { cx, FOCUS_RING } from "../../lib/cx";

export interface TabDef<T extends string> {
  id: T;
  label: string;
}

export interface TabsProps<T extends string> {
  label: string;
  tabs: readonly TabDef<T>[];
  value: T;
  onChange: (next: T) => void;
  /** Rendered once per tab id, in order; the unselected ones are `hidden`. A null body leaves the (empty) tab panel in place. */
  children: (id: T) => ReactNode;
}

/** Index to move to from `from` in direction `dir`, wrapping. */
export const nextTab = (count: number, from: number, dir: 1 | -1): number => (count <= 0 ? 0 : (from + dir + count) % count);

export function Tabs<T extends string>({ label, tabs, value, onChange, children }: TabsProps<T>) {
  const base = useId();
  const refs = useRef<(HTMLButtonElement | null)[]>([]);

  const go = (index: number) => {
    const tab = tabs[index];
    if (!tab) return;
    onChange(tab.id);
    refs.current[index]?.focus();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const rtl = getComputedStyle(e.currentTarget).direction === "rtl";
    let target = -1;
    switch (e.key) {
      case "ArrowRight":
        target = nextTab(tabs.length, index, rtl ? -1 : 1);
        break;
      case "ArrowLeft":
        target = nextTab(tabs.length, index, rtl ? 1 : -1);
        break;
      case "Home":
        target = 0;
        break;
      case "End":
        target = tabs.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    go(target);
  };

  return (
    <div className="flex flex-col gap-4">
      <div role="tablist" aria-label={label} className="flex border-b border-border">
        {tabs.map((t, i) => {
          const on = t.id === value;
          return (
            <button
              key={t.id}
              ref={(el) => {
                refs.current[i] = el;
              }}
              type="button"
              role="tab"
              id={`${base}-tab-${t.id}`}
              aria-selected={on}
              aria-controls={`${base}-panel-${t.id}`}
              tabIndex={on ? 0 : -1}
              onClick={() => go(i)}
              onKeyDown={(e) => onKeyDown(e, i)}
              className={cx(
                "-mb-px min-h-11 border border-b-0 px-4 py-2 font-mono text-xs transition-colors duration-200",
                FOCUS_RING,
                on ? "border-border bg-background text-foreground" : "border-transparent text-muted-foreground hover:text-foreground",
              )}
            >
              {t.label}
            </button>
          );
        })}
      </div>
      {tabs.map((t) => (
        <div key={t.id} role="tabpanel" id={`${base}-panel-${t.id}`} aria-labelledby={`${base}-tab-${t.id}`} hidden={t.id !== value} tabIndex={-1}>
          {children(t.id)}
        </div>
      ))}
    </div>
  );
}
