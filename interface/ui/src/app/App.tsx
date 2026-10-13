// App shell: router, mode, auth gate, sidebar, header, live region. `/pair` renders alone (no
// sidebar or header): a visitor who is not paired has nothing to navigate to yet.
import { useCallback, useEffect, useRef, useState } from "react";
import { Router, useLocation } from "wouter";
import { useServices } from "../lib/api/hooks";
import { ModeProvider } from "../lib/mode";
import { titleFor } from "../lib/nav";
import PairView from "../views/pair";
import { AuthGate, PAIR_PATH } from "./AuthGate";
import { Header, MENU_BUTTON_ID } from "./Header";
import { DESKTOP_QUERY, useMediaQuery } from "./hooks";
import { LiveRegion } from "./LiveRegion";
import { useAppLocation } from "./location";
import { AppRoutes } from "./routes";
import { Sidebar, type CloseFocus } from "./Sidebar";

const MAIN_ID = "main";

function Shell() {
  const [path] = useLocation();
  const services = useServices();
  const desktop = useMediaQuery(DESKTOP_QUERY);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const drawerActive = drawerOpen && !desktop;

  // Where focus goes once the drawer is gone (it cannot move while the main region is still inert).
  const returnFocus = useRef<CloseFocus | null>(null);
  const closeDrawer = useCallback((focus: CloseFocus) => {
    returnFocus.current = focus;
    setDrawerOpen(false);
  }, []);
  useEffect(() => {
    if (drawerActive || !returnFocus.current) return;
    const target = returnFocus.current === "menu" ? document.getElementById(MENU_BUTTON_ID) : document.getElementById(MAIN_ID);
    returnFocus.current = null;
    target?.focus();
  }, [drawerActive]);

  // Widening to a docked layout drops the drawer state; any route change closes the drawer.
  useEffect(() => {
    if (desktop) setDrawerOpen(false);
  }, [desktop]);
  useEffect(() => {
    setDrawerOpen(false);
    window.scrollTo(0, 0);
  }, [path]);

  const title = titleFor(path, services);
  useEffect(() => {
    document.title = path === "/" ? "111 node" : `${title} · 111 node`;
  }, [path, title]);

  if (path === PAIR_PATH) return <PairView />;

  return (
    <div className="lg:flex">
      <button
        type="button"
        onClick={() => document.getElementById(MAIN_ID)?.focus()}
        className="sr-only focus-visible:not-sr-only focus-visible:fixed focus-visible:start-2 focus-visible:top-2 focus-visible:z-[60] focus-visible:border focus-visible:border-foreground focus-visible:bg-background focus-visible:px-3 focus-visible:py-2 focus-visible:font-mono focus-visible:text-xs focus-visible:outline-none"
      >
        Skip to content
      </button>
      <Sidebar path={path} desktop={desktop} drawerOpen={drawerOpen} onClose={closeDrawer} />
      <div inert={drawerActive} className="flex min-h-dvh min-w-0 flex-1 flex-col">
        <Header path={path} drawerOpen={drawerActive} desktop={desktop} onMenu={() => setDrawerOpen(true)} />
        <main id={MAIN_ID} tabIndex={-1} className="mx-auto flex w-full max-w-7xl flex-1 flex-col px-4 py-6 outline-none lg:px-8">
          <AppRoutes />
        </main>
      </div>
    </div>
  );
}

export default function App() {
  return (
    <ModeProvider>
      <Router hook={useAppLocation}>
        <AuthGate />
        <Shell />
      </Router>
      <LiveRegion />
    </ModeProvider>
  );
}
