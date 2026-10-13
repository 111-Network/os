// Sticky header (spec section 5): MENU (<lg), view title, attention badge, banner slots.
import { useEffect, useRef } from "react";
import { AttentionBadge } from "../components/attention-badge";
import { Button } from "../components/button";
import { useApiMeta, useAttention, useConnection, useServices, useVersionChanged } from "../lib/api/hooks";
import { attentionSummary } from "../lib/attention";
import { cx, FOCUS_RING } from "../lib/cx";
import { titleFor } from "../lib/nav";
import { PAIR_PATH } from "./AuthGate";
import { disconnectedText } from "./format";
import { useNow } from "./hooks";
import { announce } from "./LiveRegion";
import { hrefTo } from "./location";

export const MENU_BUTTON_ID = "menu-button";

const BANNER = "flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-border px-4 py-2 font-mono text-xs lg:px-8";

export interface HeaderProps {
  path: string;
  drawerOpen: boolean;
  /** `lg` and up: the sidebar is docked, so there is no MENU button. */
  desktop: boolean;
  onMenu: () => void;
}

export function Header({ path, drawerOpen, desktop, onMenu }: HeaderProps) {
  const services = useServices();
  const attention = useAttention();
  const connection = useConnection();
  const versionChanged = useVersionChanged();
  const { needsPair } = useApiMeta();
  const { disconnected, lastEventAt } = connection;
  const now = useNow(disconnected);

  // Tell screen readers about changes the banners show (throttled to one per 2 s by the live region).
  const wasDisconnected = useRef(false);
  useEffect(() => {
    if (disconnected === wasDisconnected.current) return;
    wasDisconnected.current = disconnected;
    announce(disconnected ? "Disconnected from the node. Showing the last known state." : "Reconnected to the node.");
  }, [disconnected]);
  useEffect(() => {
    if (versionChanged) announce("The node was updated. Reload to load the new version.");
  }, [versionChanged]);

  return (
    <header className="sticky top-0 z-30 border-b border-border bg-background/80 backdrop-blur-md">
      <div className="flex h-14 items-center gap-3 px-4 lg:px-8">
        {desktop ? null : (
          <Button
            id={MENU_BUTTON_ID}
            size="sm"
            aria-haspopup="dialog"
            aria-expanded={drawerOpen}
            aria-controls="sidebar"
            onClick={onMenu}
          >
            MENU
          </Button>
        )}
        <p className="min-w-0 flex-1 truncate font-pixel-line text-lg text-foreground">{titleFor(path, services)}</p>
        {attention.count > 0 ? (
          <a
            href={hrefTo("/overview#needs-you")}
            title={attentionSummary(attention)}
            className={cx("inline-flex min-h-11 min-w-11 items-center justify-center", FOCUS_RING)}
          >
            <AttentionBadge count={attention.count} />
          </a>
        ) : null}
      </div>

      {needsPair && path !== PAIR_PATH ? (
        <div className={cx(BANNER, "text-foreground")}>
          <span>Pairing required</span>
          <a href={hrefTo(PAIR_PATH)} className={cx("underline underline-offset-4", FOCUS_RING)}>
            Enter the pair code
          </a>
        </div>
      ) : null}

      {disconnected ? (
        <div className={cx(BANNER, "bg-secondary/40 text-muted-foreground")}>
          <span aria-hidden="true" className="inline-block h-2 w-2 shrink-0 animate-blink bg-muted-foreground" />
          <span>{disconnectedText(lastEventAt, now)}</span>
        </div>
      ) : null}

      {versionChanged ? (
        <div className={cx(BANNER, "text-foreground")}>
          <span>Node updated {"—"} reload</span>
          <Button size="sm" variant="primary" onClick={() => window.location.reload()}>
            Reload
          </Button>
        </div>
      ) : null}
    </header>
  );
}
