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
