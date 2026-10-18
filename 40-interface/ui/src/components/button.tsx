import {
  useEffect,
  useRef,
  useState,
  type AnchorHTMLAttributes,
  type ButtonHTMLAttributes,
} from "react";
import { cx, FOCUS_RING } from "../lib/cx";

export type ButtonVariant = "default" | "primary" | "destructive";
export type ButtonSize = "md" | "sm";

const BASE =
  "group inline-flex min-h-11 items-center justify-center gap-2 border font-mono transition-all duration-200 disabled:pointer-events-none disabled:opacity-50 aria-disabled:cursor-default aria-disabled:opacity-50";

const SIZE: Record<ButtonSize, string> = {
  md: "px-6 py-3 text-sm",
  sm: "px-3 py-2 text-xs",
};

const VARIANT: Record<ButtonVariant, string> = {
  // CTA secondary (DESIGN.md)
  default: "border-border text-muted-foreground hover:border-foreground hover:text-foreground",
  // CTA primary: inverted, hover flips back to outline
  primary: "border-foreground bg-foreground text-background hover:bg-transparent hover:text-foreground",
  // Text-only danger: red outline, hover inversion (black on red keeps contrast above 4.5:1)
  destructive: "border-destructive text-destructive hover:bg-destructive hover:text-background",
};

export function buttonClasses(opts: { variant?: ButtonVariant; size?: ButtonSize; className?: string } = {}): string {
  const { variant = "default", size = "md", className } = opts;
  return cx(BASE, SIZE[size], VARIANT[variant], FOCUS_RING, className);
}

const ARROW = (
  <span
    aria-hidden="true"
    className="transition-transform duration-200 group-hover:translate-x-1 motion-reduce:transition-none motion-reduce:group-hover:translate-x-0"
  >
    {"->"}
  </span>
);

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /**
   * Inline two-step confirmation, never a modal. The first click swaps the button for
   * "{confirm}? [Yes] [No]" in place (focus lands on No, Escape cancels). `onClick` fires on Yes only.
   */
  confirm?: string;
  /**
   * Shows a blinking block cursor and ignores clicks. The button is marked `aria-disabled`, not `disabled`:
   * a disabled button cannot hold focus, so the keyboard user would be thrown to the page body every time
   * an action starts. The same goes for `aria-disabled` passed in directly.
   */
  loading?: boolean;
  /** Trailing `->`. Defaults to true for `primary`. */
  arrow?: boolean;
}

export function Button({
  variant = "default",
  size = "md",
  confirm,
  loading = false,
  arrow,
  type = "button",
  disabled,
  onClick,
  className,
  children,
  ...rest
}: ButtonProps) {
  const [asking, setAsking] = useState(false);
  const mainRef = useRef<HTMLButtonElement>(null);
  const noRef = useRef<HTMLButtonElement>(null);
  const wasAsking = useRef(false);
  const showArrow = arrow ?? variant === "primary";
  const soft = loading || rest["aria-disabled"] === true || rest["aria-disabled"] === "true";
  const inert = !!disabled || soft;

  useEffect(() => {
    if (asking) noRef.current?.focus();
    else if (wasAsking.current) mainRef.current?.focus();
    wasAsking.current = asking;
  }, [asking]);

  useEffect(() => {
    if (inert && asking) setAsking(false);
  }, [inert, asking]);

  if (confirm && asking) {
    return (
      <span
        role="group"
        aria-label={`Confirm: ${confirm}`}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            setAsking(false);
          }
        }}
        className={cx("inline-flex flex-wrap items-center gap-2 font-mono", size === "sm" ? "text-xs" : "text-sm", className)}
      >
        <span className="text-foreground">{confirm}?</span>
        <button
          type={type}
          className={buttonClasses({ variant, size: "sm" })}
          onClick={(e) => {
            setAsking(false);
            onClick?.(e);
          }}
        >
          Yes
        </button>
        <button ref={noRef} type="button" className={buttonClasses({ variant: "default", size: "sm" })} onClick={() => setAsking(false)}>
          No
        </button>
      </span>
    );
  }

  return (
    <button
      {...rest}
      ref={mainRef}
      // A confirmable submit button must not submit on the first click.
      type={confirm ? "button" : type}
      disabled={!!disabled}
      aria-disabled={soft || undefined}
      aria-busy={loading || undefined}
      onClick={(e) => {
        if (soft) {
          e.preventDefault(); // no submit, no action; focus stays where it is
          return;
        }
        if (confirm) setAsking(true);
        else onClick?.(e);
      }}
      className={buttonClasses({ variant, size, className })}
    >
      {children}
      {loading ? (
        <>
          <span aria-hidden="true" className="animate-blink">
            {"█"}
          </span>
          <span className="sr-only">working</span>
        </>
      ) : showArrow ? (
        ARROW
      ) : null}
    </button>
  );
}

export interface ButtonLinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  arrow?: boolean;
  /** Opens in a new tab with `noopener noreferrer`. Used for link-outs, never an iframe. */
  external?: boolean;
}

/** A link that looks like a Button (hash-route navigation, link-outs). */
export function ButtonLink({ variant = "default", size = "md", arrow, external = false, className, children, ...rest }: ButtonLinkProps) {
  const showArrow = arrow ?? variant === "primary";
  return (
    <a
      {...rest}
      {...(external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
      className={buttonClasses({ variant, size, className })}
    >
      {children}
      {showArrow ? ARROW : null}
      {external ? <span className="sr-only"> (opens in a new tab)</span> : null}
    </a>
  );
}
