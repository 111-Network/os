// Small pieces shared by the U4 views: a hash-route link and the "ask the agent" hand-off.
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { hrefTo } from "../../app/location";
import { cx, FOCUS_RING } from "../../lib/cx";

export const LINK_CLASS = cx("text-foreground underline underline-offset-4 hover:bg-foreground hover:text-background", FOCUS_RING);

export interface RouteLinkProps extends Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> {
  /** Route path without the `#`, e.g. `/services/radio`. */
  to: string;
  children?: ReactNode;
}

/** A link inside the hash router. */
export function RouteLink({ to, className, children, ...rest }: RouteLinkProps) {
  return (
    <a {...rest} href={hrefTo(to)} className={cx(LINK_CLASS, className)}>
      {children}
    </a>
  );
}

/**
 * Text a peer or a radio supplied (a contact's label, a hub's name) inside a sentence of ours: it keeps its own direction
 * (`dir="auto"`), is isolated from the words around it so right-to-left text cannot reorder them, and wraps wherever it must.
 * The caller cleans and cuts it first (`plain`, `hubName`).
 */
export function PeerText({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span dir="auto" className={cx("[unicode-bidi:isolate] break-words", className)}>
      {children}
    </span>
  );
}
