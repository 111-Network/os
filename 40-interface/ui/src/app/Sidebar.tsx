// The sidebar (spec section 5). One component, three layouts:
//   lg+ docked, 260 px, collapsible to a 56 px icon rail (state kept in localStorage 111.ui.rail);
//   <lg off-canvas drawer, closed by default, focus-trapped while open, closes on Escape / overlay / navigation.
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { Badge } from "../components/badge";
import { Button } from "../components/button";
import { Segmented } from "../components/segmented";
import { DOT_LABEL, StatusDot } from "../components/status-dot";
import { useApiMeta, useConnection, useServices, useStatus } from "../lib/api/hooks";
import { cx } from "../lib/cx";
import { useMode } from "../lib/mode";
import {
  categoryLabel,
  dotState,
  glyphOf,
  healthDot,
  isActive,
  SETTINGS_LABEL,
  visibleNav,
  type NavItem,
  type ServiceNavItem,
} from "../lib/nav";
import { hrefTo } from "./location";

export const RAIL_STORAGE_KEY = "111.ui.rail";

/** True while the element has more content below its visible part (so a cut-off row is not mistaken for the end). */
export function useMoreBelow<T extends HTMLElement>(): [RefObject<T | null>, boolean] {
  const ref = useRef<T>(null);
  const [more, setMore] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => setMore(el.scrollHeight - el.scrollTop - el.clientHeight > 2);
    update();
    el.addEventListener("scroll", update, { passive: true });
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    resize?.observe(el);
    const mutate = typeof MutationObserver === "undefined" ? null : new MutationObserver(update);
    mutate?.observe(el, { childList: true, subtree: true, attributes: true, attributeFilter: ["hidden"] });
    window.addEventListener("resize", update);
    return () => {
      el.removeEventListener("scroll", update);
      resize?.disconnect();
      mutate?.disconnect();
      window.removeEventListener("resize", update);
    };
  }, []);
  return [ref, more];
}

function readRail(): boolean {
  try {
    return window.localStorage.getItem(RAIL_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeRail(rail: boolean): void {
  try {
    window.localStorage.setItem(RAIL_STORAGE_KEY, rail ? "1" : "0");
  } catch {
    /* kept for this page load only */
  }
}

export type CloseFocus = "menu" | "main";

export interface SidebarProps {
  /** Current route path (hash stripped). */
  path: string;
  /** `lg` and up: docked. Otherwise the sidebar is the drawer. */
  desktop: boolean;
  drawerOpen: boolean;
  /** Close the drawer; `focus` says where focus returns (the MENU button, or the main region after navigating). */
  onClose: (focus: CloseFocus) => void;
}

const FOCUSABLE = 'a[href], button:not([disabled]):not([tabindex="-1"]), input:not([disabled]), [tabindex]:not([tabindex="-1"])';

const ROW =
  "group flex min-h-11 w-full items-center gap-3 px-3 text-start font-mono text-xs transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset";

function rowTone(active: boolean): string {
  return active
    ? "bg-foreground text-background focus-visible:ring-background"
    : "text-muted-foreground hover:bg-foreground hover:text-background focus-visible:ring-foreground";
}

interface NavRowProps {
  href: string;
  active: boolean;
  rail: boolean;
  label: string;
  glyph: string;
  /** Leading dot (service rows). Its screen-reader text then names the row and the visible name is hidden from AT. */
  dot?: ReactNode;
  trail?: ReactNode;
  caps?: boolean;
  onNavigate: () => void;
}

function NavRow({ href, active, rail, label, glyph, dot, trail, caps = false, onNavigate }: NavRowProps) {
  return (
    <a
      href={hrefTo(href)}
      aria-current={active ? "page" : undefined}
      title={rail ? label : undefined}
      onClick={onNavigate}
      className={cx(ROW, rowTone(active), caps && "uppercase tracking-widest", rail && "justify-center px-0")}
    >
      {rail ? (
        dot ? (
          dot
        ) : (
          <>
            <span aria-hidden="true">{glyph}</span>
            <span className="sr-only">{label}</span>
          </>
        )
      ) : (
        <>
          {dot}
          <span aria-hidden={dot ? true : undefined} className="min-w-0 flex-1 truncate">
            {label}
          </span>
          {trail}
        </>
      )}
    </a>
  );
}

function ServiceRow({
  item,
  active,
  rail,
  disconnected,
  onNavigate,
}: {
  item: ServiceNavItem;
  active: boolean;
  rail: boolean;
  disconnected: boolean;
  onNavigate: () => void;
}) {
  const state = dotState(item.state, disconnected);
  return (
    <NavRow
      href={item.href}
      active={active}
      rail={rail}
      label={item.label}
      glyph={glyphOf(item.label)}
      onNavigate={onNavigate}
      dot={<StatusDot state={state} label={false} name={item.label} />}
      trail={
        <span
          aria-hidden="true"
          className={cx(
            "shrink-0 text-[10px] uppercase tracking-widest",
            active ? "text-background" : "text-muted-foreground group-hover:text-background",
          )}
        >
          {DOT_LABEL[state]}
        </span>
      }
    />
  );
}

function SectionLabel({ rail, children }: { rail: boolean; children: ReactNode }) {
  if (rail) return <div aria-hidden="true" className="mx-3 mb-1 mt-3 border-t border-border" />;
  return (
    <p aria-hidden="true" className="px-3 pb-1 pt-5 text-[10px] uppercase tracking-widest text-muted-foreground">
      {children}
    </p>
  );
}

function GroupToggle({
  id,
  open,
  onToggle,
  children,
  caps = false,
}: {
  id: string;
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
  caps?: boolean;
}) {
  return (
    <button
      type="button"
      aria-expanded={open}
      aria-controls={id}
      onClick={onToggle}
      className={cx(ROW, rowTone(false), caps && "uppercase tracking-widest")}
    >
      <span aria-hidden="true" className="inline-block w-3 shrink-0 text-center">
        {open ? "v" : ">"}
      </span>
      <span className="min-w-0 flex-1">{children}</span>
    </button>
  );
}

function NodeChip({ rail }: { rail: boolean }) {
  const status = useStatus();
  const { disconnected } = useConnection();
  const name = status?.node.alias || status?.node.label || "node";
  const category = status?.node.category;
  const health = healthDot(status, disconnected);
  if (rail) {
    return (
      <div className="flex min-h-11 items-center justify-center border-b border-border" title={`${name}: controlling this node`}>
        <StatusDot state={health} label={false} name={`${name} health`} />
      </div>
    );
  }
  return (
    <div className="border-b border-border p-3">
      <div className="flex items-start justify-between gap-2">
        <p className="min-w-0 truncate font-mono text-sm text-foreground" title={name}>
          {name}
        </p>
        {category ? <Badge className="shrink-0">{categoryLabel(category)}</Badge> : null}
      </div>
      <div className="mt-2">
        <StatusDot state={health} name={`${name} health`} />
      </div>
      <p className="mt-1 text-[10px] uppercase tracking-widest text-muted-foreground">controlling this node</p>
    </div>
  );
}

export function Sidebar({ path, desktop, drawerOpen, onClose }: SidebarProps) {
  const [rail, setRail] = useState(readRail);
  const drawer = !desktop;
  const open = drawer && drawerOpen;
  const railed = desktop && rail;
  const { mode, setMode } = useMode();
  const services = useServices();
  const { ready } = useApiMeta();
  const { disconnected } = useConnection();
  const nav = visibleNav(mode, services);
  const advanced = mode === "advanced";
  const panelRef = useRef<HTMLDivElement>(null);
  const [scrollRef, moreBelow] = useMoreBelow<HTMLDivElement>();

  const inSettings = path === "/settings" || path.startsWith("/settings/");
  const [settingsOpen, setSettingsOpen] = useState(inSettings);
  useEffect(() => {
    if (inSettings) setSettingsOpen(true);
  }, [inSettings]);

  const inUnavailable = nav.unavailable.some((s) => isActive(s.href, path));
  const [unavailableOpen, setUnavailableOpen] = useState(false);
  useEffect(() => {
    if (inUnavailable) setUnavailableOpen(true);
  }, [inUnavailable]);

  // Drawer open: focus inside, lock page scroll, Escape closes. onClose goes through a ref so a new
  // function identity never re-runs the effect (which would pull focus back to the close button).
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (!open) return;
    document.getElementById("drawer-close")?.focus();
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      closeRef.current("menu");
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = previous;
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Focus trap: Tab and Shift+Tab wrap inside the drawer.
  const trapTab = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!open || e.key !== "Tab" || !panelRef.current) return;
    const items = [...panelRef.current.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null);
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    const current = document.activeElement;
    if (e.shiftKey && (current === first || !panelRef.current.contains(current))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && (current === last || !panelRef.current.contains(current))) {
      e.preventDefault();
      first.focus();
    }
  };

  const onNavigate = () => {
    if (drawer) onClose("main");
  };

  const toggleRail = () => {
    setRail((r) => {
      writeRail(!r);
      return !r;
    });
  };

  const row = (item: NavItem, caps = false) => (
    <li key={item.id}>
      <NavRow
        href={item.href}
        active={isActive(item.href, path)}
        rail={railed}
        label={item.label}
        glyph={glyphOf(item.label)}
        caps={caps}
        onNavigate={onNavigate}
      />
    </li>
  );

  const serviceRow = (s: ServiceNavItem) => (
    <li key={s.id}>
      <ServiceRow item={s} active={isActive(s.href, path)} rail={railed} disconnected={disconnected} onNavigate={onNavigate} />
    </li>
  );

  const serviceNote = services === null ? (ready ? "Service list unavailable" : "Loading services") : null;

  return (
    <>
      {open ? <div aria-hidden="true" className="fixed inset-0 z-40 bg-background/80 lg:hidden" onClick={() => onClose("menu")} /> : null}
      <div
        ref={panelRef}
        id="sidebar"
        role={drawer ? "dialog" : undefined}
        aria-modal={open ? true : undefined}
        aria-label={drawer ? "Menu" : undefined}
        hidden={drawer && !open}
        onKeyDown={trapTab}
        className={cx(
          "flex flex-col border-border bg-background",
          drawer ? "fixed inset-y-0 start-0 z-50 w-72 max-w-[85vw] border-e" : cx("sticky top-0 z-20 h-dvh shrink-0 border-e", railed ? "w-14" : "w-65"),
        )}
      >
        {drawer ? (
          <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
            <span className="text-[10px] uppercase tracking-widest text-muted-foreground">menu</span>
            <Button id="drawer-close" size="sm" onClick={() => onClose("menu")}>
              CLOSE
            </Button>
          </div>
        ) : null}

        <NodeChip rail={railed} />

        <nav aria-label="Main" className="flex min-h-0 flex-1 flex-col">
          <div className="relative min-h-0 flex-1">
            <div ref={scrollRef} className="h-full overflow-y-auto overflow-x-hidden py-2">
              <ul aria-label="Pages" className="flex flex-col">
                {nav.core.map((i) => row(i, true))}
              </ul>

              <SectionLabel rail={railed}>services</SectionLabel>
              {serviceNote && !railed ? <p className="px-3 py-2 text-xs text-muted-foreground">{serviceNote}</p> : null}
              <ul aria-label="Services" className="flex flex-col">
                {nav.services.map(serviceRow)}
              </ul>
              {nav.unavailable.length > 0 && !railed ? (
                <div>
                  <GroupToggle id="nav-unavailable" open={unavailableOpen} onToggle={() => setUnavailableOpen((o) => !o)}>
                    Not available on this device ({nav.unavailable.length})
                  </GroupToggle>
                  <ul id="nav-unavailable" aria-label="Not available on this device" hidden={!unavailableOpen} className="flex flex-col ps-3">
                    {nav.unavailable.map(serviceRow)}
                  </ul>
                </div>
              ) : null}
              {/* The rail has no room for a group toggle, but the pages must stay reachable: dimmed rows. */}
              {nav.unavailable.length > 0 && railed ? (
                <ul aria-label="Not available on this device" className="flex flex-col opacity-60">
                  {nav.unavailable.map(serviceRow)}
                </ul>
              ) : null}

              {nav.advanced.length > 0 ? (
                <>
                  <SectionLabel rail={railed}>advanced</SectionLabel>
                  <ul aria-label="Advanced pages" className="flex flex-col">
                    {nav.advanced.map((i) => row(i, true))}
                  </ul>
                </>
              ) : null}
            </div>
            {moreBelow ? (
              <div
                aria-hidden="true"
                className="pointer-events-none absolute inset-x-0 bottom-0 flex h-6 items-center justify-center border-t border-border bg-background/95 font-mono text-[10px] uppercase tracking-widest text-muted-foreground"
              >
                {railed ? "v" : "v more"}
              </div>
            ) : null}
          </div>

          {/* Settings stays pinned above the mode switch; its own list scrolls when the ten layer pages are open. */}
          <div className="shrink-0 border-t border-border">
            {railed ? (
              <NavRow
                href="/settings"
                active={inSettings}
                rail
                label={SETTINGS_LABEL}
                glyph={glyphOf(SETTINGS_LABEL)}
                onNavigate={onNavigate}
              />
            ) : (
              <>
                <GroupToggle id="nav-settings" caps open={settingsOpen} onToggle={() => setSettingsOpen((o) => !o)}>
                  {SETTINGS_LABEL}
                </GroupToggle>
                <ul
                  id="nav-settings"
                  aria-label={SETTINGS_LABEL}
                  hidden={!settingsOpen}
                  className="flex max-h-[30dvh] flex-col overflow-y-auto ps-3"
                >
                  {nav.settings.map((i) => row(i))}
                </ul>
              </>
            )}
          </div>
        </nav>

        <div className={cx("flex shrink-0 flex-col gap-2 border-t border-border", railed ? "items-center p-1" : "p-3")}>
          {railed ? (
            <button
              type="button"
              aria-pressed={advanced}
              aria-label="Advanced mode"
              title={advanced ? "Advanced mode on. Switch to simple." : "Simple mode. Switch to advanced."}
              onClick={() => setMode(advanced ? "simple" : "advanced")}
              className={cx(
                "min-h-11 w-full border font-mono text-[10px] tracking-widest focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-foreground",
                advanced
                  ? "border-foreground bg-foreground text-background"
                  : "border-border text-muted-foreground hover:border-foreground hover:text-foreground",
              )}
            >
              <span aria-hidden="true">{advanced ? "ADV" : "SIM"}</span>
            </button>
          ) : (
            <div className="flex items-center gap-3">
              <Segmented
                label="Interface mode"
                value={mode}
                onChange={setMode}
                options={[
                  { value: "simple", label: "SIMPLE" },
                  { value: "advanced", label: "ADVANCED" },
                ]}
              />
              {advanced ? <Badge aria-hidden="true">ADV</Badge> : null}
            </div>
          )}
          {desktop ? (
            <button
              type="button"
              aria-expanded={!rail}
              aria-controls="sidebar"
              aria-label={rail ? "Expand sidebar" : "Collapse sidebar"}
              title={rail ? "Expand sidebar" : "Collapse sidebar"}
              onClick={toggleRail}
              className="flex min-h-11 w-full items-center gap-2 px-3 font-mono text-xs text-muted-foreground hover:bg-foreground hover:text-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-foreground"
            >
              <span aria-hidden="true">{rail ? ">>" : "<<"}</span>
              {rail ? null : <span aria-hidden="true">collapse</span>}
            </button>
          ) : null}
        </div>
      </div>
    </>
  );
}
